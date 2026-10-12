//! Prints proxy sightings from the real-time proxies feed until interrupted,
//! reconnecting whenever the server closes the stream. It needs an API key
//! with the PROXY_FIREHOSE scope.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example stream
//! ```

use std::time::Duration;

use synthient::{Client, ProxyType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    tokio::select! {
        _ = print_events(&client) => Ok(()),
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

async fn print_events(client: &Client) {
    loop {
        if let Err(err) = print_stream(client).await {
            eprintln!("stream: {err}");
        }

        // The server closes each stream after about 30 minutes. Pause before
        // reconnecting so repeated failures stay under the connection rate
        // limit.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn print_stream(client: &Client) -> synthient::Result<()> {
    let mut events = client.proxies().stream().await?;
    while let Some(event) = events.next().await {
        let event = event?;
        println!(
            "{} {} {} {} {}",
            event.timestamp.map(|t| t.0).unwrap_or_default(),
            event.ip.unwrap_or_default(),
            event.provider.unwrap_or_default(),
            event
                .r#type
                .as_ref()
                .map(ProxyType::as_str)
                .unwrap_or_default(),
            event.country_code.unwrap_or_default(),
        );
    }
    Ok(())
}
