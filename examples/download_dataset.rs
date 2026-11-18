//! Downloads the newest day of the geolocation dataset as a MaxMind DB to
//! geolocation.mmdb and verifies its checksum. It needs an API key with the
//! GEOLOCATION_FEED scope.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example download_dataset
//! ```

use sha2::{Digest, Sha256};
use synthient::{Client, ExportFormat};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;
    let dataset = client.geolocation();

    let page = dataset.list_exports(Some(1), None).await?;
    let export = page.exports.first().ok_or("no geolocation export found")?;
    let mmdb = export
        .formats
        .iter()
        .find(|file| file.format == ExportFormat::Mmdb)
        .ok_or("the export has no mmdb file")?;

    let mut download = dataset
        .download(&export.id, Some(ExportFormat::Mmdb))
        .await?;
    let mut file = BufWriter::with_capacity(1 << 20, File::create("geolocation.mmdb").await?);
    let mut hash = Sha256::new();
    while let Some(chunk) = download.next().await {
        let chunk = chunk?;
        hash.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;

    if hex::encode(hash.finalize()) != mmdb.checksum {
        return Err("checksum mismatch".into());
    }
    println!(
        "saved {} ({} bytes) to geolocation.mmdb",
        export.id, mmdb.size_bytes
    );
    Ok(())
}
