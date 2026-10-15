use std::net::Ipv4Addr;
use std::time::Duration;

use synthient::{ApiError, Error, StatusCode};

use crate::server::{
    IP_INFO, Reply, Server, closed_addr, expect_api_error, json, problem, response, response_bytes,
};

async fn lookup_error(reply: impl Fn() -> Reply + Send + Sync + 'static) -> Error {
    let server = Server::start(move |_, _| reply()).await;
    let client = server.builder().max_retries(0).build().unwrap();
    client
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap_err()
}

async fn api_error(reply: impl Fn() -> Reply + Send + Sync + 'static) -> ApiError {
    expect_api_error::<()>(Err(lookup_error(reply).await))
}

#[tokio::test]
async fn decodes_problem_details() {
    let err = api_error(|| {
        problem(
            403,
            r#"{"type":"https://docs.synthient.com/errors/forbidden","title":"Forbidden","status":403,"detail":"Requires one of: PROXY_FEEDS","instance":"other"}"#,
        )
    })
    .await;

    assert_eq!(err.status, StatusCode::FORBIDDEN);
    assert_eq!(
        err.problem_type.as_deref(),
        Some("https://docs.synthient.com/errors/forbidden")
    );
    assert_eq!(err.title.as_deref(), Some("Forbidden"));
    assert_eq!(err.detail.as_deref(), Some("Requires one of: PROXY_FEEDS"));
    assert_eq!(err.request_id.as_deref(), Some("req-123"));
    assert_eq!(err.to_string(), "api returned 403 Forbidden");
}

#[tokio::test]
async fn request_id_falls_back_to_the_problem_instance() {
    let err = api_error(|| {
        response(
            404,
            &[("content-type", "application/problem+json")],
            br#"{"title":"Not Found","status":404,"instance":"abc123"}"#,
        )
    })
    .await;

    assert_eq!(err.request_id.as_deref(), Some("abc123"));
}

#[tokio::test]
async fn keeps_plain_text_errors_as_detail() {
    let err = api_error(|| {
        response(
            401,
            &[("content-type", "text/plain; charset=utf-8")],
            b"401 invalid api key\n",
        )
    })
    .await;

    assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    assert_eq!(err.detail.as_deref(), Some("401 invalid api key"));
}

#[tokio::test]
async fn html_and_empty_errors_keep_the_status() {
    let html = api_error(|| {
        response(
            502,
            &[("content-type", "text/html")],
            b"<html>Bad Gateway</html>",
        )
    })
    .await;
    let empty = api_error(|| response(500, &[], b"")).await;
    let malformed = api_error(|| problem(400, r#"{"title":"Bad Request","errors":"oops"}"#)).await;

    assert_eq!((html.status, html.detail), (StatusCode::BAD_GATEWAY, None));
    assert_eq!(
        (empty.status, empty.detail),
        (StatusCode::INTERNAL_SERVER_ERROR, None)
    );
    assert_eq!(malformed.title.as_deref(), Some("Bad Request"));
    assert!(malformed.errors.is_empty());
}

#[tokio::test]
async fn large_error_bodies_are_cut_off() {
    let err = api_error(|| {
        let body = vec![b'x'; 1 << 20];
        response(500, &[("content-type", "text/plain")], &body)
    })
    .await;

    assert_eq!(err.detail.map(|detail| detail.len()), Some(64 << 10));
}

#[tokio::test]
async fn exposes_retry_after() {
    let err = api_error(|| response(429, &[("retry-after", "7")], b"")).await;

    assert_eq!(err.retry_after, Some(Duration::from_secs(7)));
}

#[tokio::test]
async fn malformed_success_bodies_are_decode_errors() {
    for body in [
        r#"{"ip":"8.8.8.8""#,
        r#"{"network":null}"#,
        r#"{"ip":"not an ip"}"#,
        "<html>",
    ] {
        let err = lookup_error(move || json(200, body)).await;

        assert!(matches!(err, Error::Decode(_)), "{body}: {err:?}");
    }
}

#[tokio::test]
async fn oversized_success_bodies_are_rejected() {
    let err = lookup_error(|| {
        let mut body = IP_INFO.as_bytes().to_vec();
        body.resize((16 << 20) + 1, b' ');
        response(200, &[("content-type", "application/json")], &body)
    })
    .await;

    assert!(matches!(err, Error::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn truncated_bodies_are_transport_errors() {
    let err = lookup_error(|| {
        let mut bytes = response_bytes(
            200,
            &[("content-type", "application/json")],
            IP_INFO.as_bytes(),
        );
        bytes.truncate(bytes.len() - 10);
        Reply::Partial(bytes)
    })
    .await;

    assert!(matches!(err, Error::Http(_)), "{err:?}");
}

#[tokio::test]
async fn transport_errors_do_not_include_the_url() {
    let base_url = format!("http://{}/api/v4", closed_addr().await);
    let client = synthient::Client::builder("key")
        .base_url(base_url)
        .max_retries(0)
        .build()
        .unwrap();

    let err = client
        .lookup_domain("secret-domain.example", None)
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert_eq!(err.to_string(), "http request failed");
    assert!(!format!("{err:?}").contains("secret-domain"), "{err:?}");
}
