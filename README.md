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
  - [SDK Installation](#sdk-installation)
  - [Requirements](#requirements)
  - [API Keys](#api-keys)
  - [Authentication](#authentication)
  - [SDK Example Usage](#sdk-example-usage)
  - [Available Resources and Operations](#available-resources-and-operations)
  - [Streaming](#streaming)
  - [Pagination](#pagination)
  - [Downloads](#downloads)
  - [Retries](#retries)
  - [Timeouts](#timeouts)
  - [Error Handling](#error-handling)
  - [Server Selection](#server-selection)
  - [Custom HTTP Client](#custom-http-client)
  - [Debugging](#debugging)
  - [gRPC Schema](#grpc-schema)
- [Development](#development)
  - [Maturity](#maturity)
  - [Contributions](#contributions)
  - [License](#license)

## SDK Installation

```sh
cargo add synthient
cargo add tokio --features full
```

The client is asynchronous and runs on Tokio. The examples below use `#[tokio::main]`, and the download example writes with Tokio's file API, which the `full` feature includes.

The crate has these features:

| Feature      | Default | Description                                                                           |
| ------------ | ------- | ------------------------------------------------------------------------------------- |
| `rustls`     | Yes     | TLS through rustls                                                                    |
| `native-tls` | No      | TLS through the platform's native library                                             |
| `grpcschema` | No      | The [`grpcschema`](#grpc-schema) module, which fetches the gRPC API's protobuf schema |

To use the platform's TLS library instead of rustls, turn off the default features:

```sh
cargo add synthient --no-default-features --features native-tls
```

## Requirements

Rust 1.85 or later and a Tokio runtime with its time driver enabled, which `#[tokio::main]` provides.

The newest releases of some dependencies need Rust 1.88. On Rust 1.85 to 1.87, Cargo picks releases your Rust version supports only with its MSRV-aware resolver, which edition 2024 packages use by default. In an edition 2021 package, turn it on in the `[package]` table of Cargo.toml, or in `[workspace]` for a workspace:

```toml
[package]
resolver = "3"
```

## API Keys

Every request to the HTTP API needs a Synthient API key. Keys are issued from the Synthient dashboard; if you don't have one yet, [contact us](mailto:contact@synthient.com). Each key carries scopes that decide what it can call. `BASIC` covers the lookups and `account`, and each feed has one scope for its real-time stream and one for its exports, listed under [Feeds](#feeds). See [Authentication](https://docs.synthient.com/authentication) for how scopes work.

If a request fails even though you are sending a key, it is usually one of these:

- **401 Unauthorized**: the key is malformed, unknown, or revoked. Check that it was loaded from the environment and isn't truncated.
- **402 Payment Required**: the team is out of lookup credits. `account` reports the remaining balance and when it resets.
- **403 Forbidden**: the key lacks the scope for that endpoint or feed. `account` lists the key's scopes.

## Authentication

This SDK supports the following security scheme:

| Name      | Type    | Scheme             |
| --------- | ------- | ------------------ |
| `api_key` | API key | `x-api-key` header |

Pass the key to `Client::new`, or to `Client::builder` to change other settings. The client doesn't read the key from the environment itself, so load it from an environment variable or a secret manager rather than writing it into your code:

```rust
let client = synthient::Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;
```

`Client::new` returns `Error::InvalidInput` for an empty key or one that can't be sent in a header. It makes no network calls, so an invalid key shows up as a 401 on the first request. The key is sent only to the Synthient API, never to the storage host that serves export downloads, and the client's `Debug` output never shows it.

## SDK Example Usage

### Look up an IP address

Get the network, location, and risk intelligence for an IPv4 or IPv6 address. Each lookup costs one credit, and failed lookups are free.

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

`network`, `location`, and `intelligence` are `None` when the API has no data for that section. Create one client and reuse it: cloning it is cheap, and clones share one connection pool. The examples below reuse `client`. Runnable programs are in [examples](https://github.com/synthient/synthient-rust/tree/main/examples), for example `cargo run --example lookup_ip 8.8.8.8`.

### Look up many IP addresses

Look up as many as 1000 addresses in one request, billed at `ceil(n * 0.9)` credits for the `n` distinct addresses looked up. Private and reserved addresses are skipped and duplicates collapse into one result, so match results by `ip` rather than by index.

```rust
let ips = ["8.8.8.8".parse()?, "1.1.1.1".parse()?];
for result in client.lookup_ips(&ips).await? {
    if let Some(intelligence) = &result.intelligence {
        println!("{} {}", result.ip, intelligence.risk_score);
    }
}
```

More than 1000 addresses return `Error::InvalidInput` without a request.

### Look up a domain

Get the honeypot traffic observed for a domain, with its most recent events. A domain that has never been observed returns an inactive status and zero counts rather than an error. Each lookup costs one credit.

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

The second argument caps the recent events returned, from 1 to 1000. `None` uses the API default of 50.

### Check your account

Get the account that owns the API key, with the key's scopes and the team's remaining lookup credits. It costs no credits.

```rust
let account = client.account().await?;
println!(
    "{} {:?} {}",
    account.organization.name, account.scopes, account.lookup_quota.credits
);
```

## Available Resources and Operations

<details open>
<summary>Available methods</summary>

### [Client](https://docs.rs/synthient/latest/synthient/struct.Client.html)

- [`lookup_ip`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.lookup_ip) - Look up an IPv4 or IPv6 address
- [`lookup_ips`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.lookup_ips) - Look up as many as 1000 addresses in one request
- [`lookup_domain`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.lookup_domain) - Look up the honeypot traffic observed for a domain
- [`account`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.account) - Get the account that owns the API key, the key's scopes, and the team's remaining lookup credits
- [`health`](https://docs.rs/synthient/latest/synthient/struct.Client.html#method.health) - Check that the API and its dependencies are healthy

### [Feeds](https://docs.rs/synthient/latest/synthient/struct.Feed.html)

Each feed comes from a client method, such as `client.proxies()`, and has the same methods:

- [`stream`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.stream) - Open the feed's real-time event stream
- [`list_exports`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.list_exports) - List one page of the feed's Parquet exports, newest first
- [`all_exports`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.all_exports) - Stream every export, fetching pages as needed
- [`export_meta`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.export_meta) - Get an export's checksum, size, row count, and Parquet schema
- [`download`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.download) - Download an export's Parquet file
- [`download_url`](https://docs.rs/synthient/latest/synthient/struct.Feed.html#method.download_url) - Get a presigned URL for an export's Parquet file, valid for 24 hours

| Method          | Data                                          | Event type         | Stream scope            | Export scope          |
| --------------- | --------------------------------------------- | ------------------ | ----------------------- | --------------------- |
| `proxies`       | Residential, mobile, and datacenter proxies   | `ProxyEvent`       | `PROXY_FIREHOSE`        | `PROXY_FEEDS`         |
| `anonymizers`   | VPN and anonymizer IP ranges                  | `AnonymizerEvent`  | `ANONYMIZERS_STREAM`    | `ANONYMIZERS_FEED`    |
| `torrents`      | BitTorrent DHT and tracker peers              | `TorrentEvent`     | `TORRENTS_STREAM`       | `TORRENTS_FEED`       |
| `ja4t`          | JA4T TCP fingerprints                         | `Ja4tEvent`        | `JA4T_STREAM`           | `JA4T_FEED`           |
| `helios_http`   | Helios honeypot HTTP requests                 | `HeliosHttpEvent`  | `HONEYPOT_HTTP_STREAM`  | `HONEYPOT_HTTP_FEED`  |
| `helios_https`  | Helios honeypot TLS ClientHellos              | `HeliosHttpsEvent` | `HONEYPOT_HTTPS_STREAM` | `HONEYPOT_HTTPS_FEED` |
| `helios_dns`    | Helios honeypot tunnel destinations           | `HeliosDnsEvent`   | `HONEYPOT_DNS_STREAM`   | `HONEYPOT_DNS_FEED`   |
| `helios_adb`    | Helios honeypot Android Debug Bridge commands | `HeliosAdbEvent`   | `HONEYPOT_ADB_STREAM`   | `HONEYPOT_ADB_FEED`   |

</details>

## Streaming

`stream` opens a connection to a feed's real-time events and returns an [`EventStream`](https://docs.rs/synthient/latest/synthient/struct.EventStream.html). Read it with `next`, or through the `Stream` trait with the combinators from `futures` or `tokio-stream`. Dropping it closes the connection. The server keeps a stream open for up to 30 minutes and then closes it, which ends the stream with `None`. The API has no resume cursor, so call `stream` again to reconnect:

```rust
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
            // Pause after a failure so a failing stream doesn't reconnect
            // in a tight loop.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}
```

The loop stops on a 401 or 403, because reconnecting can't fix a rejected key or a missing scope. Other failures, such as a dropped connection or an event that doesn't decode, are returned once and end the stream. After a failure, the loop waits 2 seconds before reconnecting; after the server's routine close, it reconnects at once. A 429 `Error::Api` without `retry_after` means the key already has too many open streams.

Every field of a proxy, anonymizer, torrent, or JA4T event is an `Option`, because the API may omit any of them. Helios events require their core fields: `timestamp`, `tunnel_id`, `domain`, `port`, `protocol`, and `meta` on HTTP, HTTPS, and DNS events, and `session`, `sequential_id`, `command`, and `hash` on ADB events. An event that lacks one returns `Error::Decode`, which ends the stream like any other failure.

## Pagination

`list_exports` returns one page of a feed's exports, newest first. Its first argument is the page size, from 1 to 500, and `None` uses the API default of 100:

```rust
let page = client.proxies().list_exports(Some(50), None).await?;
for export in &page.exports {
    println!("{} {} {}", export.id, export.kind, export.row_count);
}
```

To get the next page, pass `page.next_cursor.as_deref()` as the second argument. It is `None` on the last page. `all_exports` does the paging for you. It returns a stream that fetches pages as you read it and stops fetching when you drop it:

```rust
let mut exports = client.proxies().all_exports(None, None);
while let Some(export) = exports.next().await {
    let export = export?;
    println!("{} {} {}", export.id, export.kind, export.row_count);
}
```

On the first page, the newest hourly export has the ID `"latest"`, a moving alias for the freshest export. To keep a stable ID, build it from the export's `date` and `hour`, such as `"2026-10-14/21"`.

## Downloads

Exports are Parquet files. Identify one by the `id` of an `Export` from a listing, by `"latest"` for the freshest one, or by a UTC day as `"YYYY-MM-DD"` or a UTC hour as `"YYYY-MM-DD/H"`, where a zero-padded hour such as `"2026-10-14/05"` also works. `download` returns a [`Download`](https://docs.rs/synthient/latest/synthient/struct.Download.html), which yields the file in chunks as they arrive:

```rust
use tokio::io::AsyncWriteExt;

let mut download = client.proxies().download("latest").await?;
let mut file = tokio::fs::File::create("proxies.parquet").await?;
while let Some(chunk) = download.next().await {
    file.write_all(&chunk?).await?;
}
file.flush().await?;
```

The file comes from a presigned storage URL that is requested without your API key. `export_meta` returns the SHA-256 checksum to verify it against. Ask both calls for the same stable ID, because `"latest"` can move to a newer export between them. `download_url` returns the presigned URL itself for tools that read Parquet over HTTPS. The URL grants access without a key for 24 hours, so treat it as a secret.

An ID in any other form returns `Error::InvalidInput` without a request. An hourly export is available until its day's daily export is rolled up, shortly after 00:30 UTC the next day. After that, requests for the hour return a 400. While a daily export is being rolled up, requests for it return a 404 with `retry_after` set.

## Retries

The client retries a failed request up to twice by default, backing off exponentially with jitter and honoring `Retry-After`:

- Account and feed requests are retried on 500, 502, 503, and 504 responses, on transport errors, and on 429 responses that carry `Retry-After`. A download's request to the storage host is not retried, and neither is a failure while reading a stream or download.
- Billed lookups (`lookup_ip`, `lookup_ips`, and `lookup_domain`) are retried only on 429 responses that carry `Retry-After`, which the rate limiter returns before doing any work, so a lookup is never charged twice. `health` is retried the same way, so an unhealthy API is reported at once.

A 429 without `Retry-After` means the key's monthly request budget is spent or it has too many open streams, so it is returned without a retry. A `Retry-After` longer than a minute is returned to you instead of waited out. When the client has a `timeout`, a retry whose wait, from backoff or `Retry-After`, would outlast it is skipped, so you get the last response or error rather than `Error::Timeout`. The SDK turns off reqwest's own retries, so the transport never resends a request.

To change the number of retries, set `max_retries` on the builder. Zero disables retries:

```rust
let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .max_retries(4)
    .build()?;
```

## Timeouts

By default, connecting gives up after 30 seconds and a read that stalls for 60 seconds fails. Streams never hit the read timeout, because the server sends a keepalive line every 15 seconds. There is no overall limit, because streams and downloads are long-lived. To set one, use `timeout` on the builder:

```rust
let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .timeout(Duration::from_secs(10))
    .build()?;
```

The timeout covers every attempt of an operation and the waits between them. For streams and downloads, it covers opening them but not reading them. An operation that runs out of time returns `Error::Timeout`. To bound a single call instead, wrap it in `tokio::time::timeout`, or drop its future to cancel it.

## Error Handling

A request that fails with a non-success HTTP status returns [`Error::Api`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Api), which holds an [`ApiError`](https://docs.rs/synthient/latest/synthient/struct.ApiError.html) with the following fields:

| Field          | Type                            | Description                                                                                      |
| -------------- | ------------------------------- | ------------------------------------------------------------------------------------------------ |
| `status`       | `StatusCode`                    | HTTP response status code, such as `404`                                                         |
| `problem_type` | `Option<String>`                | URI identifying the problem type, such as `https://docs.synthient.com/errors/validation-failed` |
| `title`        | `Option<String>`                | Short summary of the problem type                                                                |
| `detail`       | `Option<String>`                | Explanation of this occurrence. For a plain-text error, the response body                        |
| `errors`       | `BTreeMap<String, Vec<String>>` | Validation messages by request field                                                             |
| `request_id`   | `Option<String>`                | Request ID to give Synthient support                                                             |
| `retry_after`  | `Option<Duration>`              | How long the server asked you to wait before retrying                                            |

Its `Display` output contains only the HTTP status. The other fields hold untrusted server data that can be sensitive, so log them deliberately.

### Example

```rust
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

`synthient::StatusCode` is reqwest's status code type, re-exported.

### Error Types

[`Error`](https://docs.rs/synthient/latest/synthient/enum.Error.html) is `#[non_exhaustive]`, so a `match` on it needs a catch-all arm. Its variants:

- [`Api`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Api): the API, or the storage host serving a download, returned a non-success status.
- [`InvalidInput`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.InvalidInput): an argument or setting was rejected before anything was sent, such as an empty API key, more than 1000 addresses, or a malformed export ID.
- [`Http`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Http): the HTTP client could not be built, the request could not be sent, or the response could not be read. URLs are removed from these errors, because download URLs grant access to an export.
- [`Timeout`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Timeout): the operation took longer than the client's `timeout`.
- [`Decode`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Decode): a response or stream event was not the JSON the API documents.
- [`Protocol`](https://docs.rs/synthient/latest/synthient/enum.Error.html#variant.Protocol): a response broke the API's contract, such as an oversized body or a repeated pagination cursor.

<details><summary>Status codes</summary>

<br />

| Status | Meaning                                                                                                            |
| ------ | ------------------------------------------------------------------------------------------------------------------ |
| 400    | The request is invalid, such as asking for an hourly export that has been rolled up                                |
| 401    | The API key is malformed, unknown, or revoked                                                                      |
| 402    | The team is out of lookup credits                                                                                  |
| 403    | The key lacks the scope for the endpoint or feed                                                                   |
| 404    | The export does not exist, or a daily export is still being rolled up and `retry_after` is set                     |
| 422    | The input is invalid, such as a private or reserved address, a malformed domain, or a batch with no usable address |
| 429    | With `retry_after`, the rate limit was hit. Without it, the request budget is spent or too many streams are open   |
| 5xx    | The server failed. Account and feed requests are retried on 500, 502, 503, and 504                                 |

</details>

## Server Selection

### Override Server URL Per-Client

The default server is `https://api.synthient.com/api/v4`, also available as `synthient::DEFAULT_BASE_URL`. To send requests elsewhere, such as through a gateway or to a mock server in tests, set `base_url` on the builder. The URL includes the version path:

```rust
let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .base_url("http://localhost:8080/api/v4")
    .build()?;
```

`build` returns `Error::InvalidInput` for a URL that isn't http or https, or that has credentials, a query, or a fragment.

## Custom HTTP Client

The client sends requests with [reqwest](https://docs.rs/reqwest), which already honors the `HTTP_PROXY` and `HTTPS_PROXY` environment variables and the macOS and Windows proxy settings. To configure it further, for example to route requests through a specific proxy, pass a `reqwest::ClientBuilder` to `http_client`. The crate re-exports reqwest as `synthient::reqwest`, so the versions always match:

```rust
use synthient::reqwest;

let http = reqwest::Client::builder()
    .proxy(reqwest::Proxy::all("http://proxy.example.com:8080")?)
    .connect_timeout(Duration::from_secs(30))
    .read_timeout(Duration::from_secs(60));

let client = synthient::Client::builder(std::env::var("SYNTHIENT_API_KEY")?)
    .http_client(http)
    .build()?;
```

A builder you pass replaces the default one, so set the connect and read timeouts yourself, as above. Don't set a total `timeout` on it, which would also cut off streams and downloads. The SDK takes a builder rather than a client so it can turn off reqwest's redirects, which keeps the API key away from the storage host, and its retries, which keeps a billed lookup from being sent twice.

To identify your application, `user_agent("my-app/1.2")` on the builder appends a product token to the `User-Agent` header.

## Debugging

`on_response` on the builder calls a function after every API response's headers arrive, including each retry, page, stream handshake, and error response. It receives a [`ResponseMetadata`](https://docs.rs/synthient/latest/synthient/struct.ResponseMetadata.html) with the status, the request ID to give Synthient support, the operation's rate limit, and a copy of the response headers:

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

The hook runs synchronously, before the response body is read, so keep it fast. It must be `Send + Sync`, because requests can run on many tasks at once. Storage download responses are not passed to it.

> [!WARNING]
> Response headers can contain secrets, such as presigned download URLs. Avoid logging `meta.headers` in production.

## gRPC Schema

The [`grpcschema`](https://docs.rs/synthient/latest/synthient/grpcschema/index.html) module fetches the protobuf schema of the [Synthient gRPC API](https://docs.synthient.com/grpc) through gRPC server reflection. It needs the `grpcschema` feature:

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

`fetch` returns a request that runs when awaited. Before awaiting it, you can set `endpoint`, `symbols`, `timeout` for the whole fetch (30 seconds by default), or `plaintext` to skip TLS for a local development server. `schema.descriptor_set` is a `prost_types::FileDescriptorSet` with each file after the files it imports, and the module re-exports `prost` and `prost_types` for working with it. `SchemaError::explain` describes common gRPC errors in plain language, and returns `None` for the rest. Unless `plaintext` is set, the connection uses rustls with the platform's trusted certificates, whichever TLS feature the HTTP client uses.

# Development

## Maturity

This SDK follows [Semantic Versioning](https://semver.org). Breaking changes to its public API ship only in a new major version. Response types and enums are `#[non_exhaustive]`, and enums decode values this version doesn't know into an `Unknown` variant, so new API fields and values can arrive in minor versions without breaking your code.

## Contributions

Bug reports and feature requests are welcome as [GitHub issues](https://github.com/synthient/synthient-rust/issues). For questions about the API or your account, contact [Synthient support](https://docs.synthient.com/misc/support).

## License

[Apache 2.0](https://github.com/synthient/synthient-rust/blob/main/LICENSE)
