//! Tests the client against a local HTTP server.

mod client;
mod errors;
mod feed;
#[cfg(feature = "grpcschema")]
mod grpcschema;
mod lookup;
mod retry;
mod server;
mod stream;
mod transport;
