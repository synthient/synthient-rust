use futures_util::StreamExt;
use synthient::{Error, ExportKind, ParquetType, StatusCode};

use crate::server::{
    Server, closed_addr, expect_api_error, export_page, json, problem, response, truncated_json,
};

const META: &str = r#"{"id":"2026-05-03/7","stream":"honeypot_http","kind":"hourly","format":"parquet","date":1777791600,"hour":7,"rows":2,"size":10,"checksum":"ab","created_at":1778200137,"schema":{"fields":[{"name":"tunnel_id","type":"uint64"}]}}"#;

#[tokio::test]
async fn lists_one_page() {
    let server = Server::start(|_, _| {
        json(
            200,
            &export_page(&["2026-05-03/14", "2026-05-02"], Some("c1")),
        )
    })
    .await;

    let page = server
        .client()
        .proxies()
        .list_exports(Some(2), Some("abc"))
        .await
        .unwrap();

    assert_eq!(
        server.received()[0].target,
        "/api/v4/feeds/proxies/export?limit=2&cursor=abc"
    );
    assert_eq!(page.stream, "proxies");
    assert_eq!(page.exports[0].kind, ExportKind::Hourly);
    assert_eq!(page.exports[0].hour, Some(14));
    assert_eq!(page.exports[1].kind, ExportKind::Daily);
    assert_eq!(page.exports[1].hour, None);
    assert_eq!(page.next_cursor.as_deref(), Some("c1"));
}

#[tokio::test]
async fn an_empty_cursor_ends_the_listing() {
    let server = Server::start(|_, _| json(200, &export_page(&["2026-05-03"], Some("")))).await;
    let feed = server.client().proxies();

    let page = feed.list_exports(None, None).await.unwrap();
    let ids: Vec<String> = feed
        .all_exports(None, None)
        .map(|export| export.unwrap().id)
        .collect()
        .await;

    assert_eq!(page.next_cursor, None);
    assert_eq!(ids, ["2026-05-03"]);
    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn streams_every_page_including_empty_ones() {
    let server = Server::start(|index, _| match index {
        0 => json(200, &export_page(&["2026-05-04", "2026-05-03"], Some("c1"))),
        1 => json(200, &export_page(&[], Some("c2"))),
        _ => json(200, &export_page(&["2026-05-02"], None)),
    })
    .await;

    let exports = server.client().proxies().all_exports(Some(2), Some("c0"));
    let ids: Vec<String> = exports.map(|export| export.unwrap().id).collect().await;

    assert_eq!(ids, ["2026-05-04", "2026-05-03", "2026-05-02"]);
    let targets: Vec<String> = server
        .received()
        .into_iter()
        .map(|request| request.target)
        .collect();
    assert_eq!(
        targets,
        [
            "/api/v4/feeds/proxies/export?limit=2&cursor=c0",
            "/api/v4/feeds/proxies/export?limit=2&cursor=c1",
            "/api/v4/feeds/proxies/export?limit=2&cursor=c2",
        ]
    );
}

#[tokio::test]
async fn a_repeated_cursor_ends_the_stream_with_an_error() {
    let server = Server::start(|index, _| match index {
        0 => json(200, &export_page(&["2026-05-04"], Some("c1"))),
        _ => json(200, &export_page(&["2026-05-03"], Some("c1"))),
    })
    .await;

    let mut exports = server.client().proxies().all_exports(None, None);

    assert_eq!(exports.next().await.unwrap().unwrap().id, "2026-05-04");
    assert_eq!(exports.next().await.unwrap().unwrap().id, "2026-05-03");
    assert!(matches!(
        exports.next().await,
        Some(Err(Error::Protocol(_)))
    ));
    assert!(exports.next().await.is_none());
    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn a_later_page_failure_is_returned_once() {
    let server = Server::start(|index, _| match index {
        0 => json(200, &export_page(&["2026-05-04"], Some("c1"))),
        _ => problem(400, r#"{"title":"Bad Request","detail":"bad cursor"}"#),
    })
    .await;

    let mut exports = server.client().proxies().all_exports(None, None);

    assert!(exports.next().await.unwrap().is_ok());
    assert!(
        matches!(exports.next().await, Some(Err(Error::Api(api))) if api.status == StatusCode::BAD_REQUEST)
    );
    assert!(exports.next().await.is_none());
}

#[tokio::test]
async fn dropping_the_stream_stops_fetching() {
    let server =
        Server::start(|_, _| json(200, &export_page(&["2026-05-04", "2026-05-03"], Some("c1"))))
            .await;

    let mut exports = server.client().proxies().all_exports(None, None);
    exports.next().await.unwrap().unwrap();
    drop(exports);

    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn reads_export_metadata() {
    let server = Server::start(|_, _| json(200, META)).await;

    let meta = server
        .client()
        .helios_http()
        .export_meta("2026-05-03/07")
        .await
        .unwrap();

    assert_eq!(
        server.received()[0].target,
        "/api/v4/feeds/helio/http/export/2026-05-03/7/meta"
    );
    assert_eq!(meta.schema.fields[0].r#type, ParquetType::Uint64);
}

#[tokio::test(start_paused = true)]
async fn export_json_requests_retry_body_read_errors() {
    let server = Server::start(|index, request| {
        let body = if request.target.ends_with("/meta") {
            META.to_owned()
        } else {
            export_page(&["2026-05-03/7"], None)
        };
        match index {
            0 | 2 => truncated_json(&body),
            _ => json(200, &body),
        }
    })
    .await;
    let feed = server.paused_builder().build().unwrap().proxies();

    let page = feed.list_exports(None, None).await.unwrap();
    let meta = feed.export_meta("2026-05-03/7").await.unwrap();

    assert_eq!(page.exports[0].id, "2026-05-03/7");
    assert_eq!(meta.id, "2026-05-03/7");
    assert_eq!(server.received().len(), 4);
}

#[tokio::test]
async fn invalid_export_ids_send_nothing() {
    let server = Server::start(|_, _| json(200, META)).await;
    let feed = server.client().proxies();

    for id in ["", "yesterday", "2026-05-03/24", "../account/me"] {
        assert!(
            matches!(feed.export_meta(id).await, Err(Error::InvalidInput(_))),
            "{id:?}"
        );
        assert!(
            matches!(feed.download_url(id).await, Err(Error::InvalidInput(_))),
            "{id:?}"
        );
        assert!(
            matches!(feed.download(id).await, Err(Error::InvalidInput(_))),
            "{id:?}"
        );
    }
    assert!(server.received().is_empty());
}

#[tokio::test]
async fn returns_the_presigned_url() {
    let server = Server::start(|_, _| {
        response(
            307,
            &[(
                "location",
                "https://cdn.example.com/proxies.parquet?X-Amz-Signature=abc",
            )],
            b"",
        )
    })
    .await;

    let url = server
        .client()
        .proxies()
        .download_url("latest")
        .await
        .unwrap();

    assert_eq!(
        server.received()[0].target,
        "/api/v4/feeds/proxies/export/latest"
    );
    assert_eq!(
        url.as_str(),
        "https://cdn.example.com/proxies.parquet?X-Amz-Signature=abc"
    );
}

#[tokio::test]
async fn rejects_unusable_export_redirects() {
    let cases: [(u16, &[(&str, &str)]); 4] = [
        (200, &[]),
        (307, &[]),
        (307, &[("location", "ftp://cdn.example.com/file")]),
        (
            307,
            &[("location", "https://user:pass@cdn.example.com/file")],
        ),
    ];
    for (status, headers) in cases {
        let server = Server::start(move |_, _| response(status, headers, b"")).await;

        let result = server.client().proxies().download_url("latest").await;

        assert!(
            matches!(result, Err(Error::Protocol(_))),
            "{status} {headers:?}: {result:?}"
        );
    }
}

#[tokio::test]
async fn a_rolling_up_daily_export_reports_retry_after() {
    let server = Server::start(|_, _| {
        response(
            404,
            &[
                ("retry-after", "600"),
                ("content-type", "application/problem+json"),
            ],
            br#"{"title":"Not Found"}"#,
        )
    })
    .await;

    let err = expect_api_error(server.client().proxies().download_url("2026-05-03").await);

    assert_eq!(err.status, StatusCode::NOT_FOUND);
    assert_eq!(err.retry_after, Some(std::time::Duration::from_secs(600)));
}

#[tokio::test]
async fn downloads_from_storage_without_the_api_key() {
    let server = Server::start(|index, _| match index {
        0 => response(307, &[("location", "/storage/a.parquet?sig=secret")], b""),
        1 => response(302, &[("location", "/storage/b.parquet?sig=secret")], b""),
        _ => response(
            200,
            &[("content-type", "application/octet-stream")],
            b"PAR1 data PAR1",
        ),
    })
    .await;

    let mut download = server
        .client()
        .proxies()
        .download("2026-05-03")
        .await
        .unwrap();
    let mut body = Vec::new();
    while let Some(chunk) = download.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }

    assert_eq!(body, b"PAR1 data PAR1");
    assert_eq!(download.content_length(), Some(14));
    let received = server.received();
    assert_eq!(
        received[0].header("x-api-key"),
        Some(crate::server::API_KEY)
    );
    for storage in &received[1..] {
        assert_eq!(storage.header("x-api-key"), None, "{}", storage.target);
        assert!(
            storage
                .header("user-agent")
                .unwrap()
                .starts_with("synthient-rust/")
        );
    }
    assert_eq!(received[2].target, "/storage/b.parquet?sig=secret");
}

#[tokio::test]
async fn storage_failures_are_api_errors() {
    let server = Server::start(|index, _| match index {
        0 => response(307, &[("location", "/storage/a.parquet")], b""),
        _ => response(
            403,
            &[("content-type", "application/xml")],
            b"<Error>AccessDenied</Error>",
        ),
    })
    .await;

    let err = expect_api_error(server.client().proxies().download("latest").await);

    assert_eq!(err.status, StatusCode::FORBIDDEN);
    assert_eq!(server.received().len(), 2);
}

#[tokio::test]
async fn storage_redirects_are_limited() {
    let server = Server::start(|_, _| response(307, &[("location", "/storage/again")], b"")).await;

    let err = server
        .client()
        .proxies()
        .download("latest")
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Protocol(_)), "{err:?}");
    assert_eq!(server.received().len(), 12);
}

#[tokio::test]
async fn download_errors_never_include_the_presigned_url() {
    let location = format!(
        "http://{}/file.parquet?X-Amz-Signature=secret",
        closed_addr().await
    );
    let server = Server::start(move |_, _| response(307, &[("location", &location)], b"")).await;

    let err = server
        .client()
        .proxies()
        .download("latest")
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert!(!format!("{err:?} {err}").contains("secret"), "{err:?}");
}
