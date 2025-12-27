# benchy

Fast HTTP/2 benchmark tool with connection multiplexing and stream pipelining.

## Features

- **Multiple TCP connections** - Force separate connections instead of relying on pool heuristics
- **HTTP/2 stream pipelining** - Multiple concurrent streams per connection
- **Low overhead** - Lock-free stats, channel-based latency collection
- **Latency percentiles** - p50, p95, p99, and average

## Installation

```bash
cargo install --path .
```

Or build manually:

```bash
cargo build --release
./target/release/benchy --help
```

## Usage

```bash
benchy [OPTIONS] <URL>
```

### Options

| Flag | Description | Default |
|------|-------------|---------|
| `-c` | Number of TCP connections | 10 |
| `-p` | HTTP/2 streams per connection (pipeline depth) | 10 |
| `-n` | Total number of requests | 100 |
| `-d` | POST body data | None (GET) |

### Examples

```bash
# Basic: 10 connections × 10 streams = 100 concurrent requests
benchy -n 10000 http://localhost:8080

# High throughput: 20 connections × 50 streams = 1000 concurrent requests
benchy -c 20 -p 50 -n 100000 http://localhost:8080

# POST with body
benchy -c 10 -p 20 -n 5000 -d '{"key":"value"}' http://localhost:8080/api
```

### Output

```
Benchmarking http://localhost:8080 with 10 connections x 10 streams = 100 concurrency, 10000 total requests

--- Results ---
Total time:    1.234567s
Requests/sec:  8100.45
Success:       10000
Failed:        0

--- Latency ---
Avg:           12.345ms
P50:           11.234ms
P95:           18.456ms
P99:           25.789ms
```

## Notes

- Uses `http2_prior_knowledge()` - assumes server speaks HTTP/2 directly (no upgrade negotiation)
- For HTTPS with ALPN, the tool will negotiate HTTP/2 automatically
- Warns if responses come back as HTTP/1.1 instead of HTTP/2

## License

MIT
