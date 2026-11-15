<p align="center">
  <a href="https://synthient.com">
    <img src="https://raw.githubusercontent.com/synthient/synthient-rust/main/assets/logo.svg" height="96" alt="Synthient logo">
  </a>
</p>
<h3 align="center">Synthient</h3>
<p align="center">High-fidelity data on residential proxies, VPNs, and other anonymized IPs.</p>
<p align="center">
  <a href="https://docs.synthient.com">Documentation</a> ·
  <a href="https://docs.rs/synthient">API reference</a> ·
  <a href="https://docs.synthient.com/misc/support">Support</a>
</p>

# synthient-rust

The `synthient` crate is the official Rust SDK for the [Synthient API](https://docs.synthient.com): IP and domain intelligence, account details, and real-time and bulk data feeds.

[![Crates.io](https://img.shields.io/crates/v/synthient)](https://crates.io/crates/synthient)
[![docs.rs](https://img.shields.io/docsrs/synthient)](https://docs.rs/synthient)
[![test](https://github.com/synthient/synthient-rust/actions/workflows/test.yml/badge.svg)](https://github.com/synthient/synthient-rust/actions/workflows/test.yml)
[![lint](https://github.com/synthient/synthient-rust/actions/workflows/lint.yml/badge.svg)](https://github.com/synthient/synthient-rust/actions/workflows/lint.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://github.com/synthient/synthient-rust/blob/main/LICENSE)

## Table of Contents

- [synthient-rust](#synthient-rust)
  - [Installation](#installation)
  - [Quick start](#quick-start)
  - [Authentication](#authentication)
  - [Available Resources and Operations](#available-resources-and-operations)
  - [IP and domain lookups](#ip-and-domain-lookups)
  - [Account](#account)
  - [Feeds](#feeds)
  - [Errors](#errors)
  - [Retries and timeouts](#retries-and-timeouts)
  - [Configuration](#configuration)
  - [Response metadata](#response-metadata)
  - [gRPC schema](#grpc-schema)
  - [Requirements](#requirements)
  - [Versioning](#versioning)
  - [Documentation](#documentation)
  - [Contributions](#contributions)
  - [License](#license)

## Installation

```sh
cargo add synthient
cargo add tokio --features full
```

The client is asynchronous and runs on Tokio, whose `full` feature covers the examples below. The crate has these features:

| Feature      | Default | Description                               |
| ------------ | ------- | ----------------------------------------- |
| `rustls`     | Yes     | TLS through rustls                        |
| `native-tls` | No      | TLS through the platform's native library |
| `grpcschema` | No      | The [gRPC schema fetcher](#grpc-schema)   |

To use the platform's TLS library, turn off the default features:

```sh
cargo add synthient --no-default-features --features native-tls
```

## Quick start

Create a client with the API key in `SYNTHIENT_API_KEY` and look up one address:

```rust
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = synthient::Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    let info = client.lookup_ip(Ipv4Addr::new(8, 8, 8, 8)).await?;
    if let Some(intelligence) = &info.intelligence {
        println!("{} {:?}", intelligence.risk_score, intelligence.categories);
    }
    Ok(())
}
```

Reuse one client; clones are cheap and share a connection pool. The examples below reuse `client`, and runnable programs are in [examples](https://github.com/synthient/synthient-rust/tree/main/examples). The [Rust SDK guide](https://docs.synthient.com/sdks/rust) walks through the same steps.

## Authentication

Every request needs a Synthient API key. Keys are issued from the Synthient dashboard; if you don't have one, [contact us](mailto:contact@synthient.com). A key's scopes decide what it can call: `BASIC` covers the lookups and `account`, and each feed has its own scopes, listed under [Feeds](#feeds).

This SDK supports the following security scheme:

| Name      | Type    | Scheme             |
| --------- | ------- | ------------------ |
| `api_key` | API key | `x-api-key` header |

Pass the key to `Client::new`, reading it from the environment or a secret manager:

```rust
let client = synthient::Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;
```

`Client::new` makes no network calls. To check a key, call `account`, which costs no credits.

If a request fails even though you are sending a key:

- **401 Unauthorized**: the key is malformed, unknown, or revoked.
- **402 Payment Required**: the team is out of lookup credits.
- **403 Forbidden**: the key lacks the scope for that endpoint or feed.

`account` shows the key's scopes and the team's remaining credits.

See [Authentication](https://docs.synthient.com/authentication) for how keys and scopes work.

## Available Resources and Operations

<details open>
<summary>Available methods</summary>

### [Client](https://docs.rs/synthient/latest/synthient/struct.Client.html)

- [`lookup_ip`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.lookup_ip) - Look up an IPv4 or IPv6 address
- [`lookup_ips`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.lookup_ips) - Look up as many as 1000 addresses in one request
- [`lookup_domain`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.lookup_domain) - Look up the honeypot traffic observed for a domain
- [`account`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.account) - Get the key's account, scopes, and remaining lookup credits
- [`health`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.health) - Check that the API and its dependencies are healthy

### [Feed](https://docs.rs/synthient/latest/synthient/struct.Feed.html)

Each feed comes from a client method, such as `client.proxies()`, and has the same methods:

- [`stream`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.stream) - Open the feed's real-time event stream
- [`list_exports`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.list_exports) - List one page of the feed's Parquet exports, newest first
- [`all_exports`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.all_exports) - Stream every export, fetching pages as needed
- [`export_meta`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.export_meta) - Get an export's checksum, size, row count, and Parquet schema
- [`download`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.download) - Download an export's Parquet file
- [`download_url`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.download_url) - Get a presigned URL for an export's Parquet file, valid for 24 hours

</details>

## IP and domain lookups

### Look up an IP address

Get the network, location, and risk intelligence for an IPv4 or IPv6 address. Each lookup costs one credit; failed lookups are free.

```rust
use std::net::IpAddr;

let ip: IpAddr = "2001:4860:4860::8888".parse()?;
let info = client.lookup_ip(ip).await?;
if let Some(network) = &info.network {
    println!("{} {}", network.asn, network.isp);
}
if let Some(location) = &info.location {
    println!("{} {}", location.city, location.country);
}
```

`network`, `location`, and `intelligence` are `None` when the API has no data for them. Private and reserved addresses return a 422.

### Look up many IP addresses

Look up as many as 1000 addresses in one request for `ceil(n * 0.9)` credits. Private and reserved addresses are skipped and duplicates are merged, so match results by `ip`.

```rust
use std::net::IpAddr;

let ips: [IpAddr; 2] = ["8.8.8.8".parse()?, "1.1.1.1".parse()?];
for result in client.lookup_ips(&ips).await? {
    if let Some(intelligence) = &result.intelligence {
        println!("{} {}", result.ip, intelligence.risk_score);
    }
}
```

An empty slice returns an empty `Vec` without a request, and more than 1000 addresses return `Error::InvalidInput` before anything is sent.

### Look up a domain

Get the honeypot traffic observed for a domain and its most recent events. Each lookup costs one credit.

```rust
let domain = client.lookup_domain("example.com", Some(10)).await?;
println!("{} {}", domain.status, domain.stats.events_24h);
for event in &domain.recent_events {
    println!(
        "{} {} {}",
        event.timestamp.0, event.target_subdomain, event.port
    );
}
```

The second argument caps the recent events, from 1 to 1000; `None` uses the default of 50. Each `timestamp` is a [`Timestamp`](https://docs.rs/synthient/latest/synthient/struct.Timestamp.html), the Unix time as the API sends it; `to_system_time` converts it to a `SystemTime`.

See [Lookups](https://docs.synthient.com/sdks/rust/lookups) for every option, [Types](https://docs.synthient.com/sdks/rust/types) for the response fields, and the [IP API](https://docs.synthient.com/ipapi) for what the data means.

## Account

Get the key's account, scopes, and remaining lookup credits, at no cost.

```rust
let account = client.account().await?;
println!(
    "{} {:?} {}",
    account.organization.name, account.scopes, account.lookup_quota.credits
);
```

`health` checks that the API and its dependencies are healthy, at no cost. An unhealthy API returns a 503 `Error::Api`:

```rust
client.health().await?;
```

## Feeds

Each feed comes from a client method, such as `client.proxies()`, and has a real-time stream and hourly Parquet exports that roll up into daily exports. Streaming and exports need separate scopes on the API key:

| Method         | Data                                          | Event type         | Stream scope            | Export scope          |
| -------------- | --------------------------------------------- | ------------------ | ----------------------- | --------------------- |
| `proxies`      | Residential, mobile, and datacenter proxies   | `ProxyEvent`       | `PROXY_FIREHOSE`        | `PROXY_FEEDS`         |
| `anonymizers`  | VPN and anonymizer IP ranges                  | `AnonymizerEvent`  | `ANONYMIZERS_STREAM`    | `ANONYMIZERS_FEED`    |
| `torrents`     | BitTorrent DHT and tracker peers              | `TorrentEvent`     | `TORRENTS_STREAM`       | `TORRENTS_FEED`       |
| `ja4t`         | JA4T TCP fingerprints                         | `Ja4tEvent`        | `JA4T_STREAM`           | `JA4T_FEED`           |
| `helios_http`  | Helios honeypot HTTP requests                 | `HeliosHttpEvent`  | `HONEYPOT_HTTP_STREAM`  | `HONEYPOT_HTTP_FEED`  |
| `helios_https` | Helios honeypot TLS ClientHellos              | `HeliosHttpsEvent` | `HONEYPOT_HTTPS_STREAM` | `HONEYPOT_HTTPS_FEED` |
| `helios_dns`   | Helios honeypot tunnel destinations           | `HeliosDnsEvent`   | `HONEYPOT_DNS_STREAM`   | `HONEYPOT_DNS_FEED`   |
| `helios_adb`   | Helios honeypot Android Debug Bridge commands | `HeliosAdbEvent`   | `HONEYPOT_ADB_STREAM`   | `HONEYPOT_ADB_FEED`   |

See [Feeds](https://docs.synthient.com/sdks/rust/feeds) for every method, and [Firehose](https://docs.synthient.com/enterprise/firehose) and [Feeds](https://docs.synthient.com/enterprise/feeds) for the data behind the streams and exports.

### Streaming

`stream` opens a connection to a feed's real-time events and returns an [`EventStream`](https://docs.rs/synthient/latest/synthient/struct.EventStream.html). Read it with `next` or as a `Stream`, and drop it to close the connection:

```rust
use std::time::Duration;

use synthient::Error;

async fn print_proxies(client: &synthient::Client) -> synthient::Result<()> {
    let mut events = client.proxies().stream().await?;
    while let Some(event) = events.next().await {
        let event = event?;
        println!(
            "{} {}",
            event.ip.unwrap_or_default(),
            event.provider.unwrap_or_default()
        );
    }
    Ok(())
}

loop {
    match print_proxies(&client).await {
        Ok(()) => {}
        Err(Error::Api(err)) if matches!(err.status.as_u16(), 401 | 403) => {
            return Err(Error::Api(err).into());
        }
        Err(err) => {
            eprintln!("proxy stream: {err}");
            // Avoid reconnecting in a tight loop.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}
```

- The server closes each stream after up to 30 minutes. That ends the stream with `None`, and the loop reconnects at once.
- A 401 or 403 stops the loop, because reconnecting can't fix a rejected key or a missing scope.
- Any other failure ends the stream, and the loop waits 2 seconds before reconnecting.
- Events sent while disconnected are missed. The API has no resume cursor.

### Exports

`list_exports` returns one page of a feed's exports, newest first. Its first argument is the page size, from 1 to 500; `None` uses the default of 100. Pass `page.next_cursor.as_deref()` as the second to get the next page; it is `None` on the last one.

```rust
let page = client.proxies().list_exports(Some(50), None).await?;
for export in &page.exports {
    println!("{} {} {}", export.id, export.kind, export.row_count);
}
```

`all_exports` returns a stream that fetches the pages for you as you read it:

```rust
let mut exports = client.proxies().all_exports(None, None);
while let Some(export) = exports.next().await {
    let export = export?;
    println!("{} {} {}", export.id, export.kind, export.row_count);
}
```

On the first page, the newest hourly export has the ID `"latest"`, a moving alias for the freshest export. Its `date` and `hour` give its stable ID, such as `"2026-10-14/21"`.

### Downloads

Exports are Parquet files. `download` returns a [`Download`](https://docs.rs/synthient/latest/synthient/struct.Download.html), which yields the file in chunks as they arrive:

```rust
use tokio::io::AsyncWriteExt;

let mut download = client.proxies().download("latest").await?;
let mut file = tokio::fs::File::create("proxies.parquet").await?;
while let Some(chunk) = download.next().await {
    file.write_all(&chunk?).await?;
}
file.flush().await?;
```

- Name an export by its `id` from a listing, by `"latest"`, or by a UTC day as `"YYYY-MM-DD"` or hour as `"YYYY-MM-DD/H"`.
- `export_meta` returns the file's SHA-256 checksum. Use a fixed ID for both calls, because `"latest"` can move between them.
- `download_url` returns a presigned URL that works without a key for 24 hours, so treat it as a secret. Your API key is never sent to the storage host.
- Hourly exports roll into the daily export shortly after 00:30 UTC the next day. After that, the hour returns a 400.
- `download.content_length()` returns the file's size in bytes, if the storage host sent it.

## Errors

A request that fails with a non-success status returns [`Error::Api`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Api), which holds an [`ApiError`](https://docs.rs/synthient/latest/synthient/struct.ApiError.html):

| Field          | Type                            | Description                           |
| -------------- | ------------------------------- | ------------------------------------- |
| `status`       | `StatusCode`                    | HTTP status code, such as `404`       |
| `problem_type` | `Option<String>`                | URI identifying the problem type      |
| `title`        | `Option<String>`                | Short summary of the problem type     |
| `detail`       | `Option<String>`                | Explanation of this occurrence        |
| `errors`       | `BTreeMap<String, Vec<String>>` | Validation messages by request field  |
| `request_id`   | `Option<String>`                | Request ID to give Synthient support  |
| `retry_after`  | `Option<Duration>`              | How long the server asked you to wait |

Its `Display` output contains only the HTTP status, so it is safe to log. The fields hold untrusted server data.

### Example

```rust
use std::net::Ipv4Addr;

use synthient::{Error, StatusCode};

match client.lookup_ip(Ipv4Addr::new(8, 8, 8, 8)).await {
    Ok(info) => println!("{:?}", info.intelligence),
    Err(Error::Api(err)) if err.status == StatusCode::PAYMENT_REQUIRED => {
        eprintln!("out of lookup credits");
    }
    Err(Error::Api(err)) if err.status == StatusCode::TOO_MANY_REQUESTS => {
        match err.retry_after {
            Some(retry_after) => eprintln!("rate limited, retry after {retry_after:?}"),
            None => eprintln!("monthly request budget spent"),
        }
    }
    Err(Error::Api(err)) => {
        let message = err.detail.or(err.title).unwrap_or_default();
        let request_id = err.request_id.unwrap_or_default();
        eprintln!("request {request_id} failed: {message}");
    }
    Err(err) => eprintln!("lookup failed: {err}"),
}
```

### Error Types

[`Error`](https://docs.rs/synthient/latest/synthient/enum.Error.html) is `#[non_exhaustive]`, so a `match` on it needs a catch-all arm. Its variants:

- [`Api`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Api): the API, or the storage host during `download`, returned a non-success status.
- [`InvalidInput`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.InvalidInput): an argument or setting, such as an empty API key, was rejected before any request is sent.
- [`Http`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Http): the HTTP client could not be built, the request could not be sent, or the response could not be read.
- [`Timeout`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Timeout): the operation outlasted the client's `timeout`.
- [`Decode`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Decode): a response or stream event was not the JSON the API documents.
- [`Protocol`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Protocol): a response broke the API's contract, such as an oversized body.

<details><summary>Status codes</summary>

<br />

| Status | Meaning                                                                                          |
| ------ | ------------------------------------------------------------------------------------------------ |
| 400    | The request is invalid, such as an hourly export that was rolled up                              |
| 401    | The API key is malformed, unknown, or revoked                                                    |
| 402    | The team is out of lookup credits                                                                |
| 403    | The key lacks the scope for the endpoint or feed                                                 |
| 404    | The export doesn't exist, or a daily export is still rolling up (`retry_after` is set)           |
| 422    | The input is invalid, such as a private address or a malformed domain                            |
| 429    | Rate limited if `retry_after` is set; otherwise the budget is spent or too many streams are open |
| 5xx    | The server failed                                                                                |

</details>

See [Errors and retries](https://docs.synthient.com/sdks/rust/errors) for the SDK's error types and [Errors](https://docs.synthient.com/errors) for what each status means.

## Retries and timeouts

### Retries

The client retries a failed request up to twice, backing off exponentially with jitter and honoring `Retry-After`:

| Requests                    | Retried on                                                                                   |
| --------------------------- | -------------------------------------------------------------------------------------------- |
| Account and feed requests   | 500, 502, 503, and 504, transport errors, and 429 with `Retry-After`                         |
| Billed lookups and `health` | Only 429 with `Retry-After`, which comes before any work, so a lookup is never charged twice |

A 429 without `Retry-After` means the monthly request budget is spent or too many streams are open, so it is returned at once. Downloads from the storage host are never retried.

Backoff starts at one second and doubles up to 30 seconds, with 25% jitter either way. The wait is never shorter than `Retry-After`, and a `Retry-After` longer than a minute is returned to you instead of waited out. See [Rate Limits](https://docs.synthient.com/rate-limits) for the limits behind a 429.

Set the number of retries with `max_retries` on `Client::builder`. Zero disables them:

```rust
let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .max_retries(4)
    .build()?;
```

### Timeouts

By default, connecting gives up after 30 seconds and a read that stalls for 60 seconds fails. Streams send a keepalive line every 15 seconds, so they never stall that long. There is no overall timeout, because streams and downloads are long-lived. Set one with `timeout` on the builder:

```rust
use std::time::Duration;

let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .timeout(Duration::from_secs(10))
    .build()?;
```

It covers every attempt and the waits between them. For streams and downloads, it covers only opening them. An operation that runs out of time returns `Error::Timeout`. To bound a single call, wrap it in `tokio::time::timeout`.

## Configuration

`Client::builder` returns a [`ClientBuilder`](https://docs.rs/synthient/latest/synthient/struct.ClientBuilder.html). Each option is one of its methods, and `build` creates the client:

| Method        | Default                            | Description                                                      |
| ------------- | ---------------------------------- | ---------------------------------------------------------------- |
| `base_url`    | `https://api.synthient.com/api/v4` | The API base URL, including the version path                     |
| `max_retries` | `2`                                | How many times a failed request is retried; `0` disables retries |
| `timeout`     | None                               | A time limit for each operation                                  |
| `user_agent`  | None                               | A product token added to the `User-Agent` header                 |
| `http_client` | 30 s connect, 60 s read timeout    | A reqwest `ClientBuilder` to build the HTTP client from          |
| `on_response` | None                               | A function called after every API response                       |

The crate exports the default base URL as `synthient::DEFAULT_BASE_URL` and its own version as `synthient::VERSION`. See [Client](https://docs.synthient.com/sdks/rust/client) for each option in detail.

### Override the server URL

The default server is `https://api.synthient.com/api/v4`. Use `base_url` on the builder to send requests elsewhere, such as a mock server in tests:

```rust
let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .base_url("http://localhost:8080/api/v4")
    .build()?;
```

The URL must be `http` or `https`, with a host and no credentials, query, or fragment; anything else makes `build` return `Error::InvalidInput`.

### Custom HTTP client

Pass a [reqwest](https://docs.rs/reqwest) `ClientBuilder` to `http_client`, for example to use a specific proxy. Build it from the re-exported `synthient::reqwest`, so the versions match:

```rust
use std::time::Duration;

use synthient::reqwest;

let http = reqwest::Client::builder()
    .proxy(reqwest::Proxy::all("http://proxy.example.com:8080")?)
    .connect_timeout(Duration::from_secs(30))
    .read_timeout(Duration::from_secs(60));

let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .http_client(http)
    .build()?;
```

- Set the connect and read timeouts yourself, as above, because your builder replaces the defaults.
- Don't set a total `timeout` on it. It would also cut off streams and downloads.
- The SDK turns off reqwest's redirects and retries on your builder, so the API key never reaches the storage host and a billed lookup is never sent twice.
- The default client already honors `HTTP_PROXY`, `HTTPS_PROXY`, and the macOS and Windows proxy settings.

`user_agent("my-app/1.2")` on the builder adds your product token to the `User-Agent` header. The header starts with `synthient-rust/` and the crate version.

## Response metadata

`on_response` on the builder calls a function after every API response, including retries, pages, and stream handshakes, with a [`ResponseMetadata`](https://docs.rs/synthient/latest/synthient/struct.ResponseMetadata.html) holding the status, request ID, rate limit, and headers:

```rust
let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .on_response(|meta| {
        eprintln!("status={} request_id={:?}", meta.status, meta.request_id);
        if let Some(rate_limit) = &meta.rate_limit {
            eprintln!(
                "rate limit: {} of {} left",
                rate_limit.remaining, rate_limit.limit
            );
        }
    })
    .build()?;
```

The hook runs synchronously, before the response body is read, so keep it fast. It is not called for download responses from the storage host.

`meta.rate_limit` is a [`RateLimit`](https://docs.rs/synthient/latest/synthient/struct.RateLimit.html) with `limit`, `remaining`, and `reset`, the time until the bucket is full again. It is `None` when a response reports no rate limit, as on `health`.

> [!WARNING]
> Response headers can contain secrets, such as presigned download URLs. Avoid logging `meta.headers` in production.

## gRPC schema

The [`grpcschema`](https://docs.rs/synthient/latest/synthient/grpcschema/index.html) module fetches the protobuf schema of the [Synthient gRPC API](https://docs.synthient.com/grpc) through server reflection. It needs the `grpcschema` feature:

```sh
cargo add synthient --features grpcschema
```

```rust
let schema = match synthient::grpcschema::fetch(std::env::var("SYNTHIENT_API_KEY")?).await {
    Ok(schema) => schema,
    Err(err) => match err.explain() {
        Some(explanation) => return Err(format!("{explanation} {err}").into()),
        None => return Err(err.into()),
    },
};
for file in &schema.descriptor_set.file {
    println!("{} {}", file.name(), file.package());
}
```

`schema.descriptor_set` is a `prost_types::FileDescriptorSet`, and the module re-exports `prost` and `prost_types`. `SchemaError::explain` describes common gRPC errors in plain language.

Change a fetch's options before awaiting it:

```rust
use std::time::Duration;

let schema = synthient::grpcschema::fetch(std::env::var("SYNTHIENT_API_KEY")?)
    .symbols(["grpc.reflection.v1.ServerReflection"])
    .timeout(Duration::from_secs(10))
    .await?;
```

- `endpoint` sets the server. The default is `grpc.synthient.com:443`.
- `plaintext(true)` disables TLS, for local development servers.
- `symbols` resolves the named services or messages instead of every service the server lists.
- `timeout` sets how long the whole fetch may take. The default is 30 seconds.

See [gRPC schema](https://docs.synthient.com/sdks/rust/grpc-schema) for the full reference.

## Requirements

Rust 1.85 or later, and a Tokio runtime with its time driver enabled, which `#[tokio::main]` provides.

The newest releases of some dependencies need Rust 1.88, so Rust 1.85 to 1.87 needs Cargo's MSRV-aware resolver. Edition 2024 packages use it by default. In an edition 2021 package, turn it on in `[package]`, or in `[workspace]` for a workspace:

```toml
[package]
resolver = "3"
```

## Versioning

This SDK follows [Semantic Versioning](https://semver.org). Breaking changes ship only in a new major version. Response types and enums are `#[non_exhaustive]`, and unknown enum values decode to `Unknown`, so new API fields and values can arrive in minor versions.

## Documentation

- [Rust SDK guide](https://docs.synthient.com/sdks/rust): [Client](https://docs.synthient.com/sdks/rust/client), [Lookups](https://docs.synthient.com/sdks/rust/lookups), [Feeds](https://docs.synthient.com/sdks/rust/feeds), [Errors and retries](https://docs.synthient.com/sdks/rust/errors), [Types](https://docs.synthient.com/sdks/rust/types), and [gRPC schema](https://docs.synthient.com/sdks/rust/grpc-schema)
- [API reference on docs.rs](https://docs.rs/synthient)
- [Synthient API documentation](https://docs.synthient.com): [IP API](https://docs.synthient.com/ipapi), [Authentication](https://docs.synthient.com/authentication), [Rate Limits](https://docs.synthient.com/rate-limits), [Errors](https://docs.synthient.com/errors), [Feeds](https://docs.synthient.com/enterprise/feeds), and [Firehose](https://docs.synthient.com/enterprise/firehose)

## Contributions

Bug reports and feature requests are welcome as [GitHub issues](https://github.com/synthient/synthient-rust/issues). For questions about the API or your account, contact [Synthient support](https://docs.synthient.com/misc/support).

## License

[Apache 2.0](https://github.com/synthient/synthient-rust/blob/main/LICENSE)
