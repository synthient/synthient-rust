use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use synthient::{Error, StatusCode};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::server::{
    ACCOUNT, IP_INFO, Reply, Server, expect_api_error, head, json, rate_limited, response,
    truncated_json,
};

#[tokio::test(start_paused = true)]
async fn free_calls_retry_server_errors() {
    let server = Server::start(|index, _| match index {
        0 => response(503, &[], b""),
        1 => response(502, &[], b""),
        _ => json(200, ACCOUNT),
    })
    .await;

    server
        .paused_builder()
        .build()
        .unwrap()
        .account()
        .await
        .unwrap();

    assert_eq!(server.received().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn retries_stop_at_max_retries() {
    for (max_retries, attempts) in [(2, 3), (0, 1)] {
        let server = Server::start(|_, _| response(500, &[], b"")).await;
        let client = server
            .paused_builder()
            .max_retries(max_retries)
            .build()
            .unwrap();

        let err = expect_api_error(client.account().await);

        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            server.received().len(),
            attempts,
            "max_retries {max_retries}"
        );
    }
}

#[tokio::test]
async fn billed_lookups_do_not_retry_server_errors() {
    for code in [500, 502, 503, 504] {
        let server = Server::start(move |_, _| response(code, &[], b"")).await;
        let client = server.client();

        let ip = expect_api_error(client.lookup_ip(Ipv4Addr::new(8, 8, 8, 8)).await);
        let ips = expect_api_error(client.lookup_ips(&["8.8.8.8".parse().unwrap()]).await);
        let domain = expect_api_error(client.lookup_domain("example.com").await);

        assert_eq!([ip.status, ips.status, domain.status], [code; 3]);

        assert_eq!(server.received().len(), 3, "status {code}");
    }
}

#[tokio::test(start_paused = true)]
async fn billed_lookups_retry_rate_limits_after_the_requested_delay() {
    let server = Server::start(|index, _| match index {
        0 => rate_limited("5"),
        _ => json(200, IP_INFO),
    })
    .await;
    let start = Instant::now();

    server
        .paused_builder()
        .build()
        .unwrap()
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap();

    assert_eq!(server.received().len(), 2);
    assert!(
        start.elapsed() >= Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn rate_limits_without_retry_after_are_not_retried() {
    let server = Server::start(|_, _| rate_limited("")).await;
    let client = server.client();

    let lookup = expect_api_error(client.lookup_ip(Ipv4Addr::new(8, 8, 8, 8)).await);
    let account = expect_api_error(client.account().await);

    assert_eq!(
        [lookup.status, account.status],
        [StatusCode::TOO_MANY_REQUESTS; 2]
    );
    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn long_retry_after_is_returned_to_the_caller() {
    let server = Server::start(|_, _| rate_limited("120")).await;

    let err = expect_api_error(server.client().account().await);

    assert_eq!(err.retry_after, Some(Duration::from_secs(120)));
    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn retry_after_past_the_timeout_is_returned_immediately() {
    let server = Server::start(|_, _| rate_limited("30")).await;
    let client = server
        .builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let start = Instant::now();

    let err = expect_api_error(client.account().await);

    assert_eq!(err.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(server.received().len(), 1);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn free_calls_retry_transport_errors() {
    let server = Server::start(|index, _| match index {
        0 => Reply::Close,
        _ => json(200, ACCOUNT),
    })
    .await;

    server
        .paused_builder()
        .build()
        .unwrap()
        .account()
        .await
        .unwrap();

    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn billed_lookups_do_not_retry_transport_errors() {
    let server = Server::start(|_, _| Reply::Close).await;

    let err = server
        .client()
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert_eq!(server.received().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn free_calls_retry_body_read_errors() {
    let server = Server::start(|index, _| match index {
        0 => truncated_json(ACCOUNT),
        _ => json(200, ACCOUNT),
    })
    .await;
    let client = server.paused_builder().build().unwrap();

    let account = client.account().await.unwrap();

    assert_eq!(account.email, "dev@example.com");
    assert_eq!(server.received().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn body_read_errors_share_the_retry_budget() {
    for (max_retries, attempts) in [(0, 1), (1, 2), (2, 3)] {
        let server = Server::start(|index, _| match index {
            1 => response(503, &[], b""),
            _ => truncated_json(ACCOUNT),
        })
        .await;
        let client = server
            .paused_builder()
            .max_retries(max_retries)
            .build()
            .unwrap();

        let err = client.account().await.unwrap_err();

        if max_retries == 1 {
            assert!(matches!(err, Error::Api(_)), "{err:?}");
        } else {
            assert!(matches!(err, Error::Http(_)), "{err:?}");
        }
        assert_eq!(
            server.received().len(),
            attempts,
            "max_retries {max_retries}"
        );
    }
}

#[tokio::test]
async fn free_calls_retry_body_read_timeouts() {
    let (_sender, receiver) = mpsc::unbounded_channel();
    let receiver = Mutex::new(Some(receiver));
    let server = Server::start(move |index, _| match index {
        0 => Reply::Stream(
            head(200, &[("content-length", "100")]),
            receiver.lock().unwrap().take().unwrap(),
        ),
        _ => json(200, ACCOUNT),
    })
    .await;
    let client = server
        .builder()
        .http_client(synthient::reqwest::Client::builder().read_timeout(Duration::from_millis(200)))
        .max_retries(1)
        .build()
        .unwrap();

    client.account().await.unwrap();

    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn body_read_retry_past_the_timeout_is_skipped() {
    let server = Server::start(|_, _| truncated_json(ACCOUNT)).await;
    let client = server
        .builder()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();

    let err = client.account().await.unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn billed_lookups_do_not_retry_body_read_errors() {
    let server = Server::start(|_, _| truncated_json(IP_INFO)).await;
    let client = server.client();

    let ip = client
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap_err();
    let ips = client
        .lookup_ips(&["8.8.8.8".parse().unwrap()])
        .await
        .unwrap_err();
    let domain = client.lookup_domain("example.com").await.unwrap_err();

    for err in [ip, ips, domain] {
        assert!(matches!(err, Error::Http(_)), "{err:?}");
    }
    assert_eq!(server.received().len(), 3);
}
