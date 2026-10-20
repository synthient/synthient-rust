//! A minimal HTTP/1.1 server that records requests and answers with scripted
//! raw bytes, so tests control exactly when connections close and how bodies
//! are framed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use synthient::reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use synthient::{ApiError, Client, ClientBuilder, Error, StatusCode};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

pub(crate) const API_KEY: &str = "00000000-0000-4000-8000-000000000000";

#[derive(Debug, Clone)]
pub(crate) struct Received {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) connection: usize,
}

impl Received {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }
}

pub(crate) enum Reply {
    /// Writes a complete response and keeps the connection open.
    Full(Vec<u8>),
    Delayed(Duration, Vec<u8>),
    /// Writes the bytes, then closes the connection.
    Partial(Vec<u8>),
    /// Closes the connection without responding.
    Close,
    /// Writes the head, then each chunk as it is sent, and closes the
    /// connection when the sender is dropped.
    Stream(Vec<u8>, mpsc::UnboundedReceiver<Vec<u8>>),
}

type Handler = dyn Fn(usize, &Received) -> Reply + Send + Sync;

pub(crate) struct Server {
    pub(crate) base_url: String,
    pub(crate) origin: String,
    received: Arc<Mutex<Vec<Received>>>,
}

impl Server {
    /// `handler` gets each request's index, which counts every request across
    /// connections.
    pub(crate) async fn start(
        handler: impl Fn(usize, &Received) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);

        let state = received.clone();
        tokio::spawn(async move {
            for connection in 0.. {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let (read, write) = socket.into_split();
                tokio::spawn(serve(
                    read,
                    write,
                    connection,
                    state.clone(),
                    handler.clone(),
                ));
            }
        });

        Self {
            base_url: format!("{origin}/api/v4"),
            origin,
            received,
        }
    }

    pub(crate) fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    pub(crate) fn builder(&self) -> ClientBuilder {
        Client::builder(API_KEY).base_url(&self.base_url)
    }

    /// Returns a builder for tests on a paused clock, which skips backoff
    /// waits. Its client has no reqwest timeouts, because auto-advancing time
    /// would fire them while a request is in flight.
    pub(crate) fn paused_builder(&self) -> ClientBuilder {
        self.builder()
            .http_client(synthient::reqwest::Client::builder())
    }

    pub(crate) fn client(&self) -> Client {
        self.builder().build().unwrap()
    }
}

async fn serve(
    read: OwnedReadHalf,
    mut write: OwnedWriteHalf,
    connection: usize,
    received: Arc<Mutex<Vec<Received>>>,
    handler: Arc<Handler>,
) {
    let mut reader = BufReader::new(read);
    while let Some(request) = read_request(&mut reader, connection).await {
        let index = {
            let mut received = received.lock().unwrap();
            received.push(request.clone());
            received.len() - 1
        };
        match handler(index, &request) {
            Reply::Full(bytes) => {
                if write.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            Reply::Delayed(delay, bytes) => {
                tokio::time::sleep(delay).await;
                if write.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            Reply::Partial(bytes) => {
                let _ = write.write_all(&bytes).await;
                return;
            }
            Reply::Close => return,
            Reply::Stream(head, mut chunks) => {
                if write.write_all(&head).await.is_err() {
                    return;
                }
                while let Some(chunk) = chunks.recv().await {
                    if write.write_all(&chunk).await.is_err() {
                        return;
                    }
                }
                return;
            }
        }
    }
}

async fn read_request(
    reader: &mut BufReader<OwnedReadHalf>,
    connection: usize,
) -> Option<Received> {
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();

    let mut headers = HeaderMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        let name = HeaderName::from_bytes(name.trim().as_bytes()).ok()?;
        let value = HeaderValue::from_str(value.trim()).ok()?;
        headers.append(name, value);
    }

    let mut request = Received {
        method,
        target,
        headers,
        body: Vec::new(),
        connection,
    };
    let length = request
        .header("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    request.body = vec![0; length];
    reader.read_exact(&mut request.body).await.ok()?;
    Some(request)
}

pub(crate) fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Reply {
    Reply::Full(response_bytes(status, headers, body))
}

pub(crate) fn response_bytes(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let length = body.len().to_string();
    let mut headers = headers.to_vec();
    headers.push(("content-length", &length));
    let mut bytes = head(status, &headers);
    bytes.extend_from_slice(body);
    bytes
}

/// Returns a response head without framing headers. Add them with
/// `headers`, or leave them out to delimit the body by closing the
/// connection.
pub(crate) fn head(status: u16, headers: &[(&str, &str)]) -> Vec<u8> {
    let reason = StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Unknown");
    let mut text = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    text.into_bytes()
}

pub(crate) fn json(status: u16, body: &str) -> Reply {
    response(
        status,
        &[("content-type", "application/json")],
        body.as_bytes(),
    )
}

pub(crate) fn truncated_json(body: &str) -> Reply {
    let mut bytes = response_bytes(
        200,
        &[("content-type", "application/json")],
        body.as_bytes(),
    );
    bytes.pop();
    Reply::Partial(bytes)
}

pub(crate) fn problem(status: u16, body: &str) -> Reply {
    response(
        status,
        &[
            ("content-type", "application/problem+json"),
            ("x-request-id", "req-123"),
        ],
        body.as_bytes(),
    )
}

pub(crate) fn rate_limited(retry_after: &str) -> Reply {
    response(429, &[("retry-after", retry_after)], b"")
}

pub(crate) fn expect_api_error<T: std::fmt::Debug>(result: synthient::Result<T>) -> ApiError {
    match result {
        Err(Error::Api(err)) => *err,
        other => panic!("expected an API error, got {other:?}"),
    }
}

/// Returns the address of a local port that refuses connections.
pub(crate) async fn closed_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().to_string()
}

/// Starts a server that answers one request with an NDJSON stream whose
/// chunks the test sends through the returned sender. Dropping the sender
/// closes the stream.
pub(crate) async fn ndjson_server() -> (Server, mpsc::UnboundedSender<Vec<u8>>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let receiver = Mutex::new(Some(receiver));
    let server = Server::start(move |_, _| {
        let receiver = receiver.lock().unwrap().take().expect("one stream request");
        Reply::Stream(
            head(200, &[("content-type", "application/x-ndjson")]),
            receiver,
        )
    })
    .await;
    (server, sender)
}

pub(crate) const IP_INFO: &str = r#"{"ip":"8.8.8.8","network":{"asn":15169,"isp":"Google LLC","type":"DATACENTER","org":"Google LLC","domain":"google.com"},"location":{"country":"US","state":"California","city":"Mountain View","timezone":"America/Los_Angeles","latitude":37.4056,"longitude":-122.0775,"geo_hash":"9q9hvu"},"intelligence":{"risk_score":12,"categories":["DATACENTER_PROXY"],"behavior":[],"providers":[{"provider":"BRIGHTDATA","type":"DATACENTER_PROXY","last_seen":1778200137}],"devices":[{"os":"Linux","version":"6.1","last_seen":1778200137}]}}"#;

pub(crate) const ACCOUNT: &str = r#"{"email":"dev@example.com","first_name":"Ada","last_name":"Lovelace","organization":{"id":"5f0c6c1e-8a4b-4d0f-9a43-1b2f6d0e7c11","name":"Example","relation":"OWNER"},"scopes":["BASIC","PROXY_FEEDS","NEW_SCOPE"],"lookup_quota":{"credits":1000,"resets_in":3600},"api_key":{"id":"8c1f6a52-0d7e-4f39-b1d2-3e4f5a6b7c8d","name":"sdk","created_at":"2026-09-28T19:04:52.463869Z"}}"#;

pub(crate) const DOMAIN_INFO: &str = r#"{"domain":"example.com","status":"active","stats":{"events_24h":3,"total_events_30d":40},"unique_ips":{"value_24h":2,"value_30d":9,"sparkline_24h":[0,1,2]},"time_series":[{"date":1778112000,"events":3,"unique_ips":2}],"top_asn":{"asn":13335,"events":3},"top_subdomains":[{"subdomain":"www","count":3}],"top_ports":[{"port":443,"count":3}],"geo_distribution":[{"country_code":"US","events":3,"unique_ips":2}],"hour_dow_heatmap":[[0,1],[2,3]],"activity_stats":{"cadence":"Activity follows a weekend pattern.","median_per_hour":0,"p95_per_hour":2,"peak_hour":14,"quiet_hour":3},"recent_events":[{"timestamp":1778200137,"source_ip_masked":"200.13.201.29","target_subdomain":"www","country_code":"US","port":443}]}"#;

fn export(id: &str) -> String {
    let (date, hour) = id.split_once('/').unwrap_or((id, ""));
    let (kind, hour) = match hour {
        "" => ("daily", String::new()),
        hour => ("hourly", format!(r#","hour":{hour}"#)),
    };
    format!(
        r#"{{"id":"{id}","kind":"{kind}","date":"{date}"{hour},"size_bytes":10,"row_count":2,"checksum":"ab","created_at":1778200137,"download_path":"/api/v4/feeds/proxies/export/{id}"}}"#
    )
}

pub(crate) fn export_page(ids: &[&str], next_cursor: Option<&str>) -> String {
    let exports: Vec<String> = ids.iter().map(|id| export(id)).collect();
    let next = next_cursor
        .map(|cursor| format!(r#","next_cursor":"{cursor}""#))
        .unwrap_or_default();
    format!(
        r#"{{"stream":"proxies","feeds":[{}]{next}}}"#,
        exports.join(",")
    )
}
