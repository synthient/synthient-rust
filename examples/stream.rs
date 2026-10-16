//! Prints proxy sightings from the real-time proxies feed until interrupted,
//! reconnecting whenever the server closes the stream. It needs an API key
//! with the PROXY_FIREHOSE scope, and stops on errors a reconnect cannot fix.
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
    // The server closes each stream after about 30 minutes. Pause before
    // reconnecting so repeated failures stay under the connection rate
    // limit.
    let pause = Duration::from_secs(2);
    loop {
        let delay = match print_stream(client).await {
            Ok(()) => pause,
            // A client error, such as a key without the PROXY_FIREHOSE scope
            // or a 429 for too many open streams, fails the same way on every
            // attempt unless the server says when to retry.
            Err(Error::Api(err)) if err.status.is_client_error() => {
                let Some(retry_after) = err.retry_after else {
                    return Err(Error::Api(err));
                };
                eprintln!("stream: {err}");
                retry_after
            }
            Err(err) => {
                eprintln!("stream: {err}");
                pause
            }
        };
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
