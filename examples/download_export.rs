//! Lists the newest exports of the proxies feed, then downloads the most
//! recent daily export to proxies.parquet and verifies its checksum. It needs
//! an API key with the PROXY_FEEDS scope.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example download_export
//! ```

use sha2::{Digest, Sha256};
use synthient::{Client, ExportKind};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;
    let feed = client.proxies();

    let mut exports = feed.all_exports(None, None);
    let mut daily = None;
    while let Some(export) = exports.next().await {
        let export = export?;
        println!(
            "{:<16} {:<6} {:>10} rows {:>12} bytes",
            export.id, export.kind, export.row_count, export.size_bytes
        );
        if export.kind == ExportKind::Daily {
            daily = Some(export);
            break;
        }
    }
    let daily = daily.ok_or("no daily export found")?;

    let mut download = feed.download(&daily.id).await?;
    let mut file = BufWriter::with_capacity(1 << 20, File::create("proxies.parquet").await?);
    let mut hash = Sha256::new();
    while let Some(chunk) = download.next().await {
        let chunk = chunk?;
        hash.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    if hex::encode(hash.finalize()) != daily.checksum {
        return Err("checksum mismatch".into());
    }
    println!("saved {} to proxies.parquet", daily.id);
    Ok(())
}
