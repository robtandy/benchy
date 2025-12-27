use clap::Parser;
use colored::Colorize;
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::{Client, Version};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "benchy", about = "HTTP/2 and HTTP/3 benchmark tool")]
struct Args {
    /// Number of concurrent connections
    #[arg(short = 'c', default_value = "10")]
    connections: usize,

    /// Total number of requests
    #[arg(short = 'n', default_value = "100")]
    requests: u64,

    /// POST body data
    #[arg(short = 'd')]
    data: Option<String>,

    /// Pipelining depth per connection (concurrent streams)
    #[arg(short = 'p', default_value = "10")]
    pipeline: usize,

    /// Use HTTP/3 (QUIC) instead of HTTP/2
    #[arg(long = "h3")]
    http3: bool,

    /// Target URL
    url: String,
}

struct Stats {
    success: AtomicU64,
    failed: AtomicU64,
}

fn build_client(http3: bool) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder()
        .pool_max_idle_per_host(1)
        .pool_idle_timeout(Duration::from_secs(30));

    if http3 {
        builder = builder.http3_prior_knowledge();
    } else {
        builder = builder.http2_prior_knowledge();
    }

    builder.build()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let protocol = if args.http3 { "HTTP/3" } else { "HTTP/2" };
    let expected_version = if args.http3 { Version::HTTP_3 } else { Version::HTTP_2 };

    let stats = Arc::new(Stats {
        success: AtomicU64::new(0),
        failed: AtomicU64::new(0),
    });

    let (tx, mut rx) = mpsc::unbounded_channel::<Duration>();

    let url: Arc<str> = args.url.into();
    let data: Option<Arc<str>> = args.data.map(|s| s.into());

    println!(
        "{} {} ({}) with {} connections x {} streams = {} concurrency, {} total requests",
        "Benchmarking".cyan().bold(),
        url.yellow(),
        protocol.magenta(),
        args.connections.to_string().green(),
        args.pipeline.to_string().green(),
        (args.connections * args.pipeline).to_string().green().bold(),
        args.requests.to_string().green()
    );

    let start = Instant::now();

    let reqs_per_worker = args.requests / args.connections as u64;
    let remainder = args.requests % args.connections as u64;

    let mut handles = Vec::with_capacity(args.connections);

    for i in 0..args.connections {
        let client = build_client(args.http3)?;

        let url = url.clone();
        let data = data.clone();
        let stats = stats.clone();
        let tx = tx.clone();
        let pipeline = args.pipeline;

        let my_reqs = reqs_per_worker + if (i as u64) < remainder { 1 } else { 0 };

        handles.push(tokio::spawn(async move {
            let mut in_flight = FuturesUnordered::new();
            let mut sent = 0u64;

            while sent < my_reqs && in_flight.len() < pipeline {
                in_flight.push(send_request(&client, &url, &data, &stats, expected_version));
                sent += 1;
            }

            while let Some(elapsed) = in_flight.next().await {
                let _ = tx.send(elapsed);

                if sent < my_reqs {
                    in_flight.push(send_request(&client, &url, &data, &stats, expected_version));
                    sent += 1;
                }
            }
        }));
    }

    drop(tx);

    let collector = tokio::spawn(async move {
        let mut latencies = Vec::with_capacity(args.requests as usize);
        while let Some(d) = rx.recv().await {
            latencies.push(d);
        }
        latencies
    });

    for h in handles {
        let _ = h.await;
    }

    let mut latencies = collector.await?;
    let total_time = start.elapsed();

    let success = stats.success.load(Ordering::Relaxed);
    let failed = stats.failed.load(Ordering::Relaxed);

    latencies.sort_unstable();

    let len = latencies.len();
    let p50 = latencies.get(len / 2).copied().unwrap_or_default();
    let p95 = latencies.get(len * 95 / 100).copied().unwrap_or_default();
    let p99 = latencies.get(len * 99 / 100).copied().unwrap_or_default();
    let avg = if len > 0 {
        latencies.iter().sum::<Duration>() / len as u32
    } else {
        Duration::ZERO
    };

    let rps = args.requests as f64 / total_time.as_secs_f64();

    println!("\n{}", "--- Results ---".cyan().bold());
    println!("{:<14} {:?}", "Total time:".white(), total_time);
    println!("{:<14} {}", "Requests/sec:".white(), format!("{:.2}", rps).green().bold());
    println!("{:<14} {}", "Success:".white(), success.to_string().green());
    if failed > 0 {
        println!("{:<14} {}", "Failed:".white(), failed.to_string().red().bold());
    } else {
        println!("{:<14} {}", "Failed:".white(), "0".dimmed());
    }

    println!("\n{}", "--- Latency ---".cyan().bold());
    println!("{:<14} {:?}", "Avg:".white(), avg);
    println!("{:<14} {:?}", "P50:".white(), p50);
    println!("{:<14} {}", "P95:".white(), format!("{:?}", p95).yellow());
    println!("{:<14} {}", "P99:".white(), format!("{:?}", p99).red());

    Ok(())
}

#[inline]
async fn send_request(
    client: &Client,
    url: &str,
    data: &Option<Arc<str>>,
    stats: &Stats,
    expected_version: Version,
) -> Duration {
    let req_start = Instant::now();

    let result = if let Some(ref body) = data {
        client.post(url).body(body.to_string()).send().await
    } else {
        client.get(url).send().await
    };

    let elapsed = req_start.elapsed();

    match result {
        Ok(resp) => {
            if resp.version() != expected_version {
                eprintln!(
                    "{} {:?} not {:?}",
                    "Warning:".yellow(),
                    resp.version(),
                    expected_version
                );
            }
            if resp.status().is_success() {
                stats.success.fetch_add(1, Ordering::Relaxed);
            } else {
                stats.failed.fetch_add(1, Ordering::Relaxed);
            }
            let _ = resp.bytes().await;
        }
        Err(_) => {
            stats.failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    elapsed
}
