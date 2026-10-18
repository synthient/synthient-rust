//! Fetches the protobuf schema of the Synthient gRPC API through server
//! reflection and prints its services and files.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example grpc_schema --features grpcschema
//! ```

use synthient::grpcschema;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = match grpcschema::fetch(std::env::var("SYNTHIENT_API_KEY")?).await {
        Ok(schema) => schema,
        Err(err) => match err.explain() {
            Some(explanation) => return Err(format!("{explanation} {err}").into()),
            None => return Err(err.into()),
        },
    };

    println!("endpoint: {}", schema.endpoint);
    for symbol in &schema.symbols {
        println!("service: {symbol}");
    }
    for file in &schema.descriptor_set.file {
        println!("file: {} (package {})", file.name(), file.package());
    }
    Ok(())
}
