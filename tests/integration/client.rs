use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use synthient::{
    Client, Download, Error, EventStream, ExportStream, ProxyEvent, StatusCode, VERSION,
};

use crate::server::{ACCOUNT, API_KEY, Server, json, response};

#[tokio::test]
async fn sends_the_key_and_identifying_headers() {
    let server = Server::start(|_, _| json(200, ACCOUNT)).await;
    let client = server.builder().user_agent("my-app/2.0").build().unwrap();

    client.account().await.unwrap();

    let request = &server.received()[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/api/v4/account/me");
    assert_eq!(request.header("x-api-key"), Some(API_KEY));
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(
        request.header("user-agent"),
        Some(format!("synthient-rust/{VERSION} my-app/2.0").as_str())
    );
    assert_eq!(request.header("content-type"), None);
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn keeps_the_base_url_path() {
    let server = Server::start(|_, _| json(200, r#"{"status":"ok"}"#)).await;
    let client = Client::builder(API_KEY)
        .base_url(format!("{}/custom/prefix/", server.origin))
        .build()
        .unwrap();

    client.health().await.unwrap();

    assert_eq!(server.received()[0].target, "/custom/prefix/health");
}

#[tokio::test]
async fn trims_the_api_key() {
    let server = Server::start(|_, _| json(200, ACCOUNT)).await;
    let client = Client::builder(format!("  {API_KEY}\n"))
        .base_url(&server.base_url)
        .build()
        .unwrap();

    client.account().await.unwrap();

    assert_eq!(server.received()[0].header("x-api-key"), Some(API_KEY));
}

#[tokio::test(start_paused = true)]
async fn response_hook_sees_every_attempt() {
    let server = Server::start(|index, _| match index {
        0 => response(503, &[("x-request-id", "first")], b""),
        _ => response(
            200,
            &[
                ("content-type", "application/json"),
                ("x-request-id", "second"),
                ("ratelimit-limit", "10"),
                ("ratelimit-remaining", "9"),
                ("ratelimit-reset", "1"),
            ],
            ACCOUNT.as_bytes(),
        ),
    })
    .await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hook_seen = seen.clone();
    let client = server
        .paused_builder()
        .on_response(move |meta| {
            let rate_limit = meta
                .rate_limit
                .map(|limit| (limit.limit, limit.remaining, limit.reset));
            hook_seen
                .lock()
                .unwrap()
                .push((meta.status, meta.request_id.clone(), rate_limit));
        })
        .build()
        .unwrap();

    client.account().await.unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(
        *seen,
        [
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Some("first".to_owned()),
                None
            ),
            (
                StatusCode::OK,
                Some("second".to_owned()),
                Some((10, 9, Duration::from_secs(1)))
            ),
        ]
    );
}

#[tokio::test]
async fn clones_share_the_connection_pool() {
    let server = Server::start(|_, _| json(200, ACCOUNT)).await;
    let client = server.client();

    client.account().await.unwrap();
    client.clone().account().await.unwrap();

    let received = server.received();
    assert_eq!(received[0].connection, received[1].connection);
}

#[test]
fn rejects_invalid_configuration() {
    assert!(matches!(Client::new(""), Err(Error::InvalidInput(_))));
    assert!(matches!(
        Client::builder(API_KEY).base_url("not a url").build(),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        Client::builder(API_KEY).user_agent("bad\r\nagent").build(),
        Err(Error::InvalidInput(_))
    ));
}

fn assert_send<T: Send>(_: &T) {}

fn assert_send_static<T: Send + 'static>(_: &T) {}

#[test]
fn futures_and_streams_are_send() {
    fn send_sync<T: Send + Sync + 'static>() {}
    fn send<T: Send + 'static>() {}
    send_sync::<Client>();
    send_sync::<Error>();
    send::<EventStream<ProxyEvent>>();
    send::<ExportStream>();
    send::<Download>();

    let client = Client::new(API_KEY).unwrap();
    let feed = client.proxies();
    assert_send(&client.lookup_ip(Ipv4Addr::LOCALHOST));
    assert_send(&client.lookup_ips(&[]));
    assert_send(&client.account());
    assert_send(&feed.stream());
    assert_send(&feed.download("latest"));
    assert_send(&client.lookup_domain("example.com", None));
    assert_send_static(&feed.list_exports().into_future());
    assert_send_static(&feed.list_exports().into_stream());
}
