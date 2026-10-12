//! Prints the account that owns the API key, its scopes, and its remaining
//! lookup credits.
//!
//! ```sh
//! SYNTHIENT_API_KEY=... cargo run --example account
//! ```

use synthient::{Client, Scope};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("SYNTHIENT_API_KEY")?)?;

    let account = client.account().await?;

    let scopes: Vec<&str> = account.scopes.iter().map(Scope::as_str).collect();
    println!(
        "{} {} <{}>",
        account.first_name, account.last_name, account.email
    );
    println!("organization: {}", account.organization.name);
    println!("scopes: {}", scopes.join(", "));
    println!(
        "credits: {} (resets in {}s)",
        account.lookup_quota.credits, account.lookup_quota.resets_in
    );
    Ok(())
}
