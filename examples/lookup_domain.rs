//! Prints the honeypot traffic Synthient has observed for a domain. Each
//! lookup costs one credit.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example lookup_domain example.com
//! ```

use synthient::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let domain = std::env::args()
        .nth(1)
        .ok_or("usage: lookup_domain <domain>")?;
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    let info = client.lookup_domain(&domain, Some(5)).await?;

    println!("domain: {} {}", info.domain, info.status);
    println!(
        "events: {} in 24h, {} in 30d",
        info.stats.events_24h, info.stats.total_events_30d
    );
    println!(
        "unique IPs: {} in 24h, {} in 30d",
        info.unique_ips.value_24h, info.unique_ips.value_30d
    );
    for subdomain in &info.top_subdomains {
        println!(
            "subdomain: {} ({} events)",
            subdomain.subdomain, subdomain.count
        );
    }
    for event in &info.recent_events {
        println!(
            "event: {} {} -> {}:{}",
            event.timestamp.0, event.source_ip, event.target_subdomain, event.port
        );
    }
    Ok(())
}
