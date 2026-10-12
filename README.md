# synthient-rust

![Synthient](https://raw.githubusercontent.com/synthient/synthient-rust/main/assets/readme-header.svg)

[![Crates.io](https://img.shields.io/crates/v/synthient)](https://crates.io/crates/synthient)
[![docs.rs](https://img.shields.io/docsrs/synthient)](https://docs.rs/synthient)
[![test](https://github.com/synthient/synthient-rust/actions/workflows/test.yml/badge.svg)](https://github.com/synthient/synthient-rust/actions/workflows/test.yml)
[![lint](https://github.com/synthient/synthient-rust/actions/workflows/lint.yml/badge.svg)](https://github.com/synthient/synthient-rust/actions/workflows/lint.yml)

The official Rust client for the [Synthient API](https://docs.synthient.com): IP and domain intelligence, account details, and real-time and bulk data feeds.

## Install

```sh
cargo add synthient
cargo add tokio --features macros,rt-multi-thread
```

Requires Rust 1.85 or later and a Tokio runtime.

## Usage

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

Look up many addresses in one request:

```rust
let ips = ["8.8.8.8".parse()?, "1.1.1.1".parse()?];
for result in client.lookup_ips(&ips).await? {
    if let Some(intelligence) = &result.intelligence {
        println!("{} {}", result.ip, intelligence.risk_score);
    }
}
```

Consume a real-time feed:

```rust
let mut events = client.proxies().stream().await?;
while let Some(event) = events.next().await {
    let event = event?;
    println!("{} {}", event.ip.unwrap_or_default(), event.provider.unwrap_or_default());
}
```

## Documentation

The full reference, with examples for errors, retries, exports, and downloads, is on [docs.rs](https://docs.rs/synthient). Runnable programs are in [examples](https://github.com/synthient/synthient-rust/tree/main/examples), for example `cargo run --example lookup_ip 8.8.8.8`. Guides and field references are at [docs.synthient.com](https://docs.synthient.com). The `grpcschema` feature adds a module that fetches the protobuf schema of the Synthient gRPC API; enable it with `cargo add synthient --features grpcschema`.

## License

[Apache 2.0](https://github.com/synthient/synthient-rust/blob/main/LICENSE)
