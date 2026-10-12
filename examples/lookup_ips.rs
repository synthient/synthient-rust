//! Looks up several IP addresses in one batch request and prints the risk
//! score and categories of each.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example lookup_ips 8.8.8.8 1.1.1.1
//! ```

use std::net::IpAddr;

use synthient::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ips = std::env::args()
        .skip(1)
        .map(|arg| arg.parse())
        .collect::<Result<Vec<IpAddr>, _>>()?;
    if ips.is_empty() {
        return Err("usage: lookup_ips <ip>...".into());
    }
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    for result in client.lookup_ips(&ips).await? {
        match &result.intelligence {
            Some(intelligence) => println!(
                "{:<39} risk {:>3}  {}",
                result.ip,
                intelligence.risk_score,
                intelligence.categories.join(", ")
            ),
            None => println!("{:<39} no intelligence available", result.ip),
        }
    }
    Ok(())
}
