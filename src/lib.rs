//! The Rust client for the [Synthient API](https://docs.synthient.com): IP
//! and domain intelligence, account details, and the Synthient data feeds.
//!
//! Create a [`Client`] with an API key and reuse it for every request:
//!
//! ```no_run
//! use std::net::Ipv4Addr;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = synthient::Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;
//!
//! let info = client.lookup_ip(Ipv4Addr::new(8, 8, 8, 8)).await?;
//! if let Some(intelligence) = &info.intelligence {
//!     println!("{} {:?}", intelligence.risk_score, intelligence.categories);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The client is asynchronous and runs on Tokio. Cloning it is cheap, and
//! every future and stream it returns is `Send`.
//!
//! # Errors
//!
//! A request that fails with a non-success HTTP status returns
//! [`Error::Api`], whose [`ApiError`] holds the status, the problem detail,
//! the request ID, and any Retry-After delay:
//!
//! ```no_run
//! # async fn run(client: synthient::Client) {
//! use synthient::{Error, StatusCode};
//!
//! match client.lookup_ips(&["8.8.8.8".parse().unwrap()]).await {
//!     Err(Error::Api(err)) if err.status == StatusCode::PAYMENT_REQUIRED => {
//!         eprintln!("out of lookup credits");
//!     }
//!     Err(err) => eprintln!("lookup failed: {err}"),
//!     Ok(results) => println!("{} results", results.len()),
//! }
//! # }
//! ```
//!
//! Error messages contain only the HTTP status. Server-provided details are
//! in the fields of [`ApiError`] and can contain sensitive data.
//! [`ClientBuilder::on_response`] exposes request IDs and rate limits for
//! every API response.
//!
//! # Retries
//!
//! Requests that cost no credits are retried on 500, 502, 503, and 504
//! responses, on transport errors, and on 429 responses that carry
//! Retry-After. Billed lookups ([`Client::lookup_ip`], [`Client::lookup_ips`],
//! and [`Client::lookup_domain`]) are retried only on 429 responses that
//! carry Retry-After, which the rate limiter returns before doing any work. A
//! 429 without Retry-After means the key's monthly request budget is spent or
//! it has too many open streams, so it is returned without a retry.
//! [`Client::health`] is retried the same way as billed lookups, so it reports
//! an unhealthy API right away.
//!
//! Retries back off exponentially with jitter and honor Retry-After. A
//! Retry-After over a minute, or one that would outlast the client's
//! [`timeout`](ClientBuilder::timeout), is returned to the caller instead.
//! Set the retry count with [`ClientBuilder::max_retries`]. The SDK turns off
//! reqwest's own retries, so no request is ever resent by the transport.
//!
//! # Feeds
//!
//! Each [`Feed`], such as [`Client::proxies`], has a real-time stream and
//! hourly parquet exports that roll up into daily exports:
//!
//! ```no_run
//! # async fn run(client: synthient::Client) -> Result<(), synthient::Error> {
//! let mut events = client.proxies().stream().await?;
//! while let Some(event) = events.next().await {
//!     let event = event?;
//!     println!("{:?} {:?}", event.ip, event.provider);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`Feed::download`] fetches an export's parquet file without sending the
//! API key to the storage host.
//!
//! # Timeouts
//!
//! By default, connecting gives up after 30 seconds and a read that stalls
//! for 60 seconds fails. Streams send a keepalive line every 15 seconds, so
//! they never stall that long. There is no overall limit, because streams
//! and downloads are long-lived. Set one with [`ClientBuilder::timeout`], or
//! drop a future to cancel it.
//!
//! # Features
//!
//! - `rustls` (default): TLS through rustls.
//! - `native-tls`: TLS through the platform's native library.
//! - `grpcschema`: the `grpcschema` module, which fetches the protobuf
//!   schema of the Synthient gRPC API.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
mod wire_enum;

mod account;
mod client;
mod error;
mod events;
mod feed;
#[cfg(feature = "grpcschema")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpcschema")))]
pub mod grpcschema;
mod lookup;
mod response;
mod retry;
mod stream;
mod timestamp;

#[cfg(test)]
mod conformance;

pub use account::{Account, AccountApiKey, LookupQuota, Organization, Relation, Scope};
pub use client::{Client, ClientBuilder, DEFAULT_BASE_URL, VERSION};
pub use error::{ApiError, Error, Result};
pub use events::{
    AnonymizerEvent, HeliosAdbEvent, HeliosDnsEvent, HeliosHttpEvent, HeliosHttpsEvent, HeliosMeta,
    HttpRequestDetails, Ja4tEvent, ProxyEvent, ProxyType, TlsClientHello, TlsCodeName,
    TlsExtension, TorrentEvent, TorrentFile, TorrentPeer,
};
pub use feed::{
    Download, Export, ExportKind, ExportMeta, ExportPage, ExportStream, Feed, ParquetColumn,
    ParquetSchema, ParquetType,
};
pub use lookup::{
    ActivityStats, AsnCount, Cadence, CountryCount, Device, DomainEvent, DomainInfo, DomainStats,
    DomainStatus, Intelligence, IpInfo, Location, Network, NetworkType, Os, PortCount, Provider,
    SubdomainCount, TimeSeriesPoint, UniqueIps,
};
pub use reqwest;
pub use reqwest::StatusCode;
pub use response::{RateLimit, ResponseMetadata};
pub use stream::EventStream;
pub use timestamp::Timestamp;
