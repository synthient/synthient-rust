use std::time::Duration;

use synthient::{Error, ProxyType, StatusCode};

use crate::server::{
    Reply, Server, expect_api_error, head, ndjson_server, problem, response, response_bytes,
};

#[tokio::test]
async fn reads_events_and_skips_keepalives() {
    let (server, events) = ndjson_server().await;
    events
        .send(b"{\"ip\":\"1.1.1.1\",\"type\":\"RESIDENTIAL_PROXY\"}\n\n".to_vec())
        .unwrap();
    events.send(b"{\"ip\":\"2.2.2.".to_vec()).unwrap();
    events
        .send(b"2\",\"provider\":\"x\"}\r\n\n".to_vec())
        .unwrap();
    drop(events);

    let mut stream = server.client().proxies().stream().await.unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.ip.as_deref(), Some("1.1.1.1"));
    assert_eq!(first.r#type, Some(ProxyType::ResidentialProxy));
    let second = stream.next().await.unwrap().unwrap();
    assert_eq!(second.ip.as_deref(), Some("2.2.2.2"));
    assert_eq!(second.provider.as_deref(), Some("x"));
    assert!(stream.next().await.is_none());
    assert!(stream.next().await.is_none());

    let request = &server.received()[0];
    assert_eq!(request.target, "/api/v4/feeds/proxies/stream");
    assert_eq!(request.header("accept"), Some("application/x-ndjson"));
}

#[tokio::test]
async fn delivers_events_before_the_stream_ends() {
    let (server, events) = ndjson_server().await;
    let mut stream = server.client().helios_dns().stream().await.unwrap();

    events
        .send(br#"{"timestamp":1778200137487,"tunnel_id":18446744073709551615,"domain":"example.com","port":53,"protocol":"dns","meta":{"provider":"x"}}"#.to_vec())
        .unwrap();
    events.send(b"\n".to_vec()).unwrap();
    let event = stream.next().await.unwrap().unwrap();

    assert_eq!(event.tunnel_id, u64::MAX);
    assert_eq!(event.timestamp.0, 1_778_200_137_487);
    drop(events);
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_final_line_without_newline_is_still_read() {
    let (server, events) = ndjson_server().await;
    events.send(b"{\"ip\":\"1.1.1.1\"}".to_vec()).unwrap();
    drop(events);

    let mut stream = server.client().proxies().stream().await.unwrap();

    assert_eq!(
        stream.next().await.unwrap().unwrap().ip.as_deref(),
        Some("1.1.1.1")
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_truncated_event_is_an_error() {
    let (server, events) = ndjson_server().await;
    events
        .send(b"{\"ip\":\"1.1.1.1\"}\n{\"ip\":\"2.2".to_vec())
        .unwrap();
    drop(events);

    let mut stream = server.client().proxies().stream().await.unwrap();

    assert!(stream.next().await.unwrap().is_ok());
    assert!(matches!(stream.next().await, Some(Err(Error::Decode(_)))));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_malformed_event_ends_the_stream() {
    let (server, events) = ndjson_server().await;
    events
        .send(b"{\"asn\":\"not a number\"}\n{\"ip\":\"1.1.1.1\"}\n".to_vec())
        .unwrap();

    let mut stream = server.client().proxies().stream().await.unwrap();

    assert!(matches!(stream.next().await, Some(Err(Error::Decode(_)))));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn an_oversized_event_is_an_error() {
    let (server, events) = ndjson_server().await;
    events.send(vec![b'x'; (16 << 20) + 1]).unwrap();

    let mut stream = server.client().proxies().stream().await.unwrap();

    assert!(matches!(stream.next().await, Some(Err(Error::Protocol(_)))));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn an_oversized_complete_event_is_an_error() {
    let (server, events) = ndjson_server().await;
    let mut event = b"{\"provider\":\"".to_vec();
    event.resize((16 << 20) - 1, b'x');
    event.extend_from_slice(b"\"}\n");
    events.send(event).unwrap();
    drop(events);

    let mut stream = server.client().proxies().stream().await.unwrap();

    assert!(matches!(stream.next().await, Some(Err(Error::Protocol(_)))));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_dropped_connection_is_an_error() {
    let server = Server::start(|_, _| {
        let mut bytes = head(
            200,
            &[
                ("content-type", "application/x-ndjson"),
                ("content-length", "1000"),
            ],
        );
        bytes.extend_from_slice(b"{\"ip\":\"1.1.1.1\"}\n");
        Reply::Partial(bytes)
    })
    .await;

    let mut stream = server.client().proxies().stream().await.unwrap();

    assert!(stream.next().await.unwrap().is_ok());
    assert!(matches!(stream.next().await, Some(Err(Error::Http(_)))));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn open_failures_are_api_errors() {
    let server = Server::start(|_, _| {
        problem(
            403,
            r#"{"title":"Forbidden","detail":"Requires one of: PROXY_FIREHOSE"}"#,
        )
    })
    .await;

    let err = expect_api_error(server.client().proxies().stream().await);

    assert_eq!(err.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn too_many_streams_is_not_retried() {
    let server = Server::start(|_, _| response(429, &[], b"")).await;

    let err = expect_api_error(server.client().proxies().stream().await);

    assert_eq!(err.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(err.retry_after, None);
    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn the_timeout_does_not_cut_off_an_open_stream() {
    let (server, events) = ndjson_server().await;
    let client = server
        .builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let mut stream = client.proxies().stream().await.unwrap();

    tokio::time::sleep(Duration::from_millis(400)).await;
    events.send(b"{\"ip\":\"1.1.1.1\"}\n".to_vec()).unwrap();
    drop(events);

    assert!(stream.next().await.unwrap().is_ok());
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn the_timeout_bounds_an_operation() {
    let server = Server::start(|_, _| {
        Reply::Delayed(Duration::from_secs(5), response_bytes(200, &[], b"{}"))
    })
    .await;
    let client = server
        .builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let start = std::time::Instant::now();

    let err = client.account().await.unwrap_err();

    assert!(matches!(err, Error::Timeout), "{err:?}");
    assert!(start.elapsed() < Duration::from_secs(2));
}
