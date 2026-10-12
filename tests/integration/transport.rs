//! Proves that a request the server may have handled is never resent by the
//! HTTP stack, which could charge a billed lookup twice.

use std::net::Ipv4Addr;

use synthient::Error;

use crate::server::{ACCOUNT, IP_INFO, Reply, Server, expect_api_error, json, response};

#[tokio::test]
async fn billed_lookup_is_not_resent_on_a_reused_connection() {
    // The first request warms a pooled connection. The second is read, then
    // the connection closes without a response. Any later request would be
    // a resend.
    let server = Server::start(|index, _| match index {
        0 => json(200, ACCOUNT),
        1 => Reply::Close,
        _ => json(200, IP_INFO),
    })
    .await;
    let client = server.client();
    client.account().await.unwrap();

    let err = client
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    let received = server.received();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].connection, received[1].connection);
}

#[tokio::test]
async fn free_call_is_not_resent_by_the_transport() {
    let server = Server::start(|index, _| match index {
        1 => Reply::Close,
        _ => json(200, ACCOUNT),
    })
    .await;
    let client = server.builder().max_retries(0).build().unwrap();
    client.account().await.unwrap();

    let err = client.account().await.unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn injected_clients_do_not_resend_either() {
    let server = Server::start(|index, _| match index {
        0 => json(200, ACCOUNT),
        1 => Reply::Close,
        _ => json(200, IP_INFO),
    })
    .await;
    let client = server
        .builder()
        .http_client(synthient::reqwest::Client::builder())
        .build()
        .unwrap();
    client.account().await.unwrap();

    client
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap_err();

    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn injected_clients_do_not_follow_api_redirects() {
    let server = Server::start(|index, _| match index {
        0 => response(302, &[("location", "/elsewhere")], b""),
        _ => json(200, ACCOUNT),
    })
    .await;
    let client = server
        .builder()
        .http_client(synthient::reqwest::Client::builder())
        .build()
        .unwrap();

    let err = expect_api_error(client.account().await);

    assert_eq!(err.status, 302);
    assert_eq!(server.received().len(), 1);
}
