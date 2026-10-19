//! Prints proxy sightings from the real-time proxies feed until interrupted.
//! It reconnects at once when the server closes the stream, and after a pause
//! when the stream fails. It stops on a 401 or 403, which a reconnect cannot
//! fix. It needs an API key with the PROXY_FIREHOSE scope.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example stream
//! ```

use std::time::Duration;

use synthient::{Client, Error, ProxyType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    tokio::select! {
        result = print_events(&client) => Ok(result?),
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

async fn print_events(client: &Client) -> synthient::Result<()> {
    let pause = Duration::from_secs(2);
    loop {
        let delay = match print_stream(client).await {
            Ok(()) => continue,
            Err(Error::Api(err)) if matches!(err.status.as_u16(), 401 | 403) => {
                return Err(Error::Api(err));
            }
            Err(Error::Api(err)) => {
                eprintln!("stream: {err}");
                err.retry_after.unwrap_or_default().max(pause)
            }
            Err(err) => {
                eprintln!("stream: {err}");
                pause
            }
        };
        // Avoid reconnecting in a tight loop.
        tokio::time::sleep(delay).await;
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
