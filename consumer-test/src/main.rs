//! Uses the packaged crate the way a consumer would: a lookup against a local
//! server, run on a spawned task to prove the future is `Send + 'static`, and
//! with the grpcschema feature, a schema fetch from a closed port.

use std::net::Ipv4Addr;

use synthient::{Client, Error, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BODY: &str = r#"{"ip":"8.8.8.8","intelligence":{"risk_score":12,"categories":["DATACENTER_PROXY"],"behavior":[],"providers":[],"devices":[]}}"#;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/api/v4", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        assert!(request.starts_with(b"GET /api/v4/lookup/ip/8.8.8.8 HTTP/1.1\r\n"));
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{BODY}",
            BODY.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let client = Client::builder("00000000-0000-4000-8000-000000000000")
        .base_url(base_url)
        .build()
        .unwrap();
    let lookup = tokio::spawn(async move { client.lookup_ip(Ipv4Addr::new(8, 8, 8, 8)).await });
    let info = lookup.await.unwrap().unwrap();
    assert_eq!(info.intelligence.unwrap().risk_score, 12);

    let err = Client::new(" ").unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    assert_eq!(StatusCode::PAYMENT_REQUIRED, 402);

    #[cfg(feature = "grpcschema")]
    {
        use synthient::grpcschema::{self, Code};

        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = closed.local_addr().unwrap().to_string();
        drop(closed);
        let fetch = grpcschema::fetch("key").endpoint(endpoint).plaintext(true);
        let err = tokio::spawn(fetch.into_future())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.code, Some(Code::Unavailable));
    }
    println!("consumer check passed");
}
