use clap::Parser;
use colored::Colorize;
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::{Client, Version};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

    /// Skip TLS certificate verification
    #[arg(short = 'k', long = "insecure")]
    insecure: bool,

    /// Abort on first error and show details
    #[arg(short = 'f', long = "fail-fast")]
    fail_fast: bool,

    /// Target URL
    url: String,
}

struct Stats {
    success: AtomicU64,
    failed: AtomicU64,
}

#[derive(Debug)]
struct ErrorDetails {
    message: String,
    status: Option<u16>,
    headers: Option<String>,
    body: Option<String>,
}

fn build_client(http3: bool, insecure: bool, is_https: bool) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder()
        .pool_max_idle_per_host(1)
        .pool_idle_timeout(Duration::from_secs(30));

    if http3 {
        // HTTP/3 always uses QUIC (encrypted)
        builder = builder.http3_prior_knowledge();
    } else if is_https {
        // HTTPS: use ALPN negotiation for HTTP/2
        builder = builder.use_rustls_tls();
    } else {
        // Plain HTTP: use h2c (HTTP/2 over cleartext)
        builder = builder.http2_prior_knowledge();
    }

    if insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder.build()
}

enum RequestResult {
    Success(Duration),
    Failed(Duration),
    Error(ErrorDetails),
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

    let (tx, mut rx) = mpsc::unbounded_channel::<RequestResult>();
    let abort_flag = Arc::new(AtomicBool::new(false));

    let is_https = args.url.starts_with("https://");
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
        let client = build_client(args.http3, args.insecure, is_https)?;

        let url = url.clone();
        let data = data.clone();
        let stats = stats.clone();
        let tx = tx.clone();
        let pipeline = args.pipeline;
        let abort_flag = abort_flag.clone();
        let fail_fast = args.fail_fast;

        let my_reqs = reqs_per_worker + if (i as u64) < remainder { 1 } else { 0 };

        handles.push(tokio::spawn(async move {
            let mut in_flight = FuturesUnordered::new();
            let mut sent = 0u64;

            while sent < my_reqs && in_flight.len() < pipeline && !abort_flag.load(Ordering::Relaxed) {
                in_flight.push(send_request(&client, &url, &data, &stats, expected_version, fail_fast));
                sent += 1;
            }

            while let Some(result) = in_flight.next().await {
                if abort_flag.load(Ordering::Relaxed) {
                    break;
                }

                let should_abort = matches!(&result, RequestResult::Error(_));
                let _ = tx.send(result);

                if should_abort {
                    break;
                }

                if sent < my_reqs && !abort_flag.load(Ordering::Relaxed) {
                    in_flight.push(send_request(&client, &url, &data, &stats, expected_version, fail_fast));
                    sent += 1;
                }
            }
        }));
    }

    drop(tx);

    let abort_flag_collector = abort_flag.clone();
    let fail_fast = args.fail_fast;
    let collector = tokio::spawn(async move {
        let mut latencies = Vec::with_capacity(args.requests as usize);
        let mut first_error: Option<ErrorDetails> = None;

        while let Some(result) = rx.recv().await {
            match result {
                RequestResult::Success(d) | RequestResult::Failed(d) => {
                    latencies.push(d);
                }
                RequestResult::Error(details) => {
                    if fail_fast && first_error.is_none() {
                        first_error = Some(details);
                        abort_flag_collector.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        (latencies, first_error)
    });

    for h in handles {
        let _ = h.await;
    }

    let (mut latencies, first_error) = collector.await?;
    let total_time = start.elapsed();

    // Show error details if we aborted
    if let Some(err) = first_error {
        println!("\n{}", "--- Error Details ---".red().bold());
        println!("{:<14} {}", "Error:".white(), err.message.red());
        if let Some(status) = err.status {
            println!("{:<14} {}", "Status:".white(), status.to_string().yellow());
        }
        if let Some(headers) = err.headers {
            println!("\n{}:", "Headers".white().bold());
            println!("{}", headers.dimmed());
        }
        if let Some(body) = err.body {
            println!("\n{}:", "Body".white().bold());
            println!("{}", body);
        }
        std::process::exit(1);
    }

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
    fail_fast: bool,
) -> RequestResult {
    let req_start = Instant::now();

    let result = if let Some(ref body) = data {
        client
            .post(url)
            .version(expected_version)
            .body(body.to_string())
            .send()
            .await
    } else {
        client.get(url).version(expected_version).send().await
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

            let status = resp.status();
            if status.is_success() {
                stats.success.fetch_add(1, Ordering::Relaxed);
                let _ = resp.bytes().await;
                RequestResult::Success(elapsed)
            } else {
                stats.failed.fetch_add(1, Ordering::Relaxed);

                if fail_fast {
                    let headers = format!("{:#?}", resp.headers());
                    let body = resp.text().await.ok();
                    RequestResult::Error(ErrorDetails {
                        message: format!("HTTP {} {}", status.as_u16(), status.canonical_reason().unwrap_or("")),
                        status: Some(status.as_u16()),
                        headers: Some(headers),
                        body,
                    })
                } else {
                    let _ = resp.bytes().await;
                    RequestResult::Failed(elapsed)
                }
            }
        }
        Err(e) => {
            stats.failed.fetch_add(1, Ordering::Relaxed);

            if fail_fast {
                RequestResult::Error(ErrorDetails {
                    message: e.to_string(),
                    status: e.status().map(|s| s.as_u16()),
                    headers: None,
                    body: None,
                })
            } else {
                RequestResult::Failed(elapsed)
            }
        }
    }
}
