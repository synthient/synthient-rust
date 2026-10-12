//! Prints what Synthient knows about an IP address. Each lookup costs one
//! credit.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example lookup_ip 8.8.8.8
//! ```

use std::net::IpAddr;

use synthient::{Client, Error, StatusCode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ip: IpAddr = std::env::args()
        .nth(1)
        .ok_or("usage: lookup_ip <ip>")?
        .parse()?;
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    let info = match client.lookup_ip(ip).await {
        Ok(info) => info,
        Err(Error::Api(err)) if err.status == StatusCode::PAYMENT_REQUIRED => {
            return Err("out of lookup credits".into());
        }
        Err(err) => return Err(err.into()),
    };

    println!("ip: {}", info.ip);
    if let Some(network) = &info.network {
        println!(
            "network: {} (AS{}, {})",
            network.isp, network.asn, network.r#type
        );
    }
    if let Some(location) = &info.location {
        println!("location: {}, {}", location.city, location.country);
    }
    let Some(intelligence) = &info.intelligence else {
        println!("no intelligence available");
        return Ok(());
    };
    println!("risk score: {}", intelligence.risk_score);
    println!("categories: {}", intelligence.categories.join(", "));
    for provider in &intelligence.providers {
        println!(
            "provider: {} ({}), last seen at Unix time {}",
            provider.name, provider.r#type, provider.last_seen.0
        );
    }
    Ok(())
}
