# benchy

Fast HTTP/2 and HTTP/3 benchmark tool with connection multiplexing and stream pipelining.

## Features

- **HTTP/2 and HTTP/3 support** - Benchmark both protocols with `--h3` flag
- **Multiple connections** - Force separate connections instead of relying on pool heuristics
- **Stream pipelining** - Multiple concurrent streams per connection
- **Low overhead** - Lock-free stats, channel-based latency collection
- **Latency percentiles** - p50, p95, p99, and average
- **Fail-fast mode** - Abort on first error with detailed diagnostics

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
| `-c` | Number of connections | 10 |
| `-p` | Streams per connection (pipeline depth) | 10 |
| `-n` | Total number of requests | 100 |
| `-d` | POST body data | None (GET) |
| `--h3` | Use HTTP/3 (QUIC) instead of HTTP/2 | false |
| `-k, --insecure` | Skip TLS certificate verification | false |
| `-f, --fail-fast` | Abort on first error and show details | false |

### Examples

```bash
# HTTP/2 over plaintext (h2c): 10 connections × 10 streams = 100 concurrent requests
benchy -n 10000 http://localhost:8080

# HTTP/2 high throughput: 20 connections × 50 streams = 1000 concurrent requests
benchy -c 20 -p 50 -n 100000 http://localhost:8080

# HTTP/2 over TLS (HTTPS)
benchy -c 10 -p 50 -n 10000 https://localhost:8443

# HTTP/2 over TLS with self-signed cert
benchy -k -c 10 -p 50 -n 10000 https://localhost:8443

# HTTP/3 (QUIC)
benchy --h3 -c 10 -p 50 -n 10000 https://localhost:8443

# POST with body
benchy -c 10 -p 20 -n 5000 -d '{"key":"value"}' http://localhost:8080/api

# Debug mode - stop on first error and show details
benchy -f -n 100 http://localhost:8080
```

### Output

```
Benchmarking http://localhost:8080 (HTTP/2) with 10 connections x 10 streams = 100 concurrency, 10000 total requests

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

### Fail-fast Output

When using `-f`, errors show full details:

```
--- Error Details ---
Error:         HTTP 500 Internal Server Error
Status:        500

Headers:
{ "content-type": "application/json", ... }

Body:
{"error": "Something went wrong"}
```

## Notes

- `http://` URLs use h2c (HTTP/2 over cleartext, no TLS)
- `https://` URLs use HTTP/2 via ALPN negotiation
- `--h3` requires HTTPS and a QUIC-capable server
- `-k` only applies to HTTPS connections (ignores cert errors)

## License

MIT
