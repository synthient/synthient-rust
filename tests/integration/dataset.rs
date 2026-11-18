use futures_util::StreamExt;
use synthient::{Error, ExportFormat, ExportKind, WhoisEntity};

use crate::server::{API_KEY, Received, Server, export_page, json, response};

const META: &str = r#"{"id":"2026-11-17","stream":"whois","kind":"daily","format":"tar.gz","date":1794873600,"rows":2,"size":10,"checksum":"ab","created_at":1794960929,"schema":{"fields":null},"formats":[{"format":"tar.gz","size_bytes":10,"checksum":"ab"}],"entities":[{"entity":"asns","row_count":2,"size_bytes":5,"checksum":"cd","schema":{"fields":[{"name":"asn","type":"int64"}]}}]}"#;

fn targets(received: Vec<Received>) -> Vec<String> {
    received.into_iter().map(|request| request.target).collect()
}

#[tokio::test]
async fn lists_exports() {
    let server = Server::start(|index, _| match index {
        2 => json(200, &export_page(&["2026-11-17"], Some("c1"))),
        _ => json(200, &export_page(&["2026-11-16"], None)),
    })
    .await;
    let client = server.client();

    let page = client
        .geolocation()
        .list_exports(Some(5), None)
        .await
        .unwrap();
    client
        .whois()
        .list_exports_on("2026-11-17", None, Some("c0"))
        .await
        .unwrap();
    let ids: Vec<String> = client
        .whois()
        .all_exports_on("2026-11-17", Some(1), None)
        .map(|export| export.unwrap().id)
        .collect()
        .await;

    assert_eq!(page.exports[0].kind, ExportKind::Daily);
    assert_eq!(ids, ["2026-11-17", "2026-11-16"]);
    assert_eq!(
        targets(server.received()),
        [
            "/api/v4/feeds/geolocation/export?limit=5",
            "/api/v4/feeds/whois/export?cursor=c0&day=2026-11-17",
            "/api/v4/feeds/whois/export?limit=1&day=2026-11-17",
            "/api/v4/feeds/whois/export?limit=1&cursor=c1&day=2026-11-17",
        ]
    );
}

#[tokio::test]
async fn format_is_sent_only_when_set() {
    let server = Server::start(|_, request| {
        if request.target.contains("/meta") {
            json(200, META)
        } else {
            response(
                307,
                &[("location", "https://cdn.example.com/geo.mmdb?sig=secret")],
                b"",
            )
        }
    })
    .await;
    let client = server.client();

    let meta = client
        .whois()
        .export_meta("2026-11-17", None)
        .await
        .unwrap();
    client
        .geolocation()
        .export_meta("latest", Some(ExportFormat::Csv))
        .await
        .unwrap();
    let url = client
        .geolocation()
        .download_url("2026-11-17", Some(ExportFormat::Mmdb))
        .await
        .unwrap();
    client
        .geolocation()
        .download_url("latest", None)
        .await
        .unwrap();

    assert_eq!(
        targets(server.received()),
        [
            "/api/v4/feeds/whois/export/2026-11-17/meta",
            "/api/v4/feeds/geolocation/export/latest/meta?format=csv",
            "/api/v4/feeds/geolocation/export/2026-11-17?format=mmdb",
            "/api/v4/feeds/geolocation/export/latest",
        ]
    );
    assert_eq!(url.as_str(), "https://cdn.example.com/geo.mmdb?sig=secret");
    assert!(meta.schema.fields.is_empty());
    assert_eq!(meta.entities.unwrap()[0].entity, WhoisEntity::Asns);
}

#[tokio::test]
async fn reads_the_export_volume() {
    let server = Server::start(|_, _| {
        json(
            200,
            r#"{"stream":"whois","days":[{"date":"2026-11-17","kind":"daily","snapshots":1,"row_count":23481094,"size_bytes":997234948}]}"#,
        )
    })
    .await;

    let volume = server
        .client()
        .whois()
        .export_volume(Some(30))
        .await
        .unwrap();

    assert_eq!(
        server.received()[0].target,
        "/api/v4/feeds/whois/export/daily?days=30"
    );
    assert_eq!(volume.days[0].row_count, 23_481_094);
}

#[tokio::test]
async fn hourly_ids_send_nothing() {
    let server = Server::start(|_, _| json(200, META)).await;
    let dataset = server.client().geolocation();

    for id in ["2026-11-17/3", "2026-11-17/03", "daily", "../account/me"] {
        assert!(
            matches!(
                dataset.export_meta(id, None).await,
                Err(Error::InvalidInput(_))
            ),
            "{id:?}"
        );
        assert!(
            matches!(
                dataset.download_url(id, None).await,
                Err(Error::InvalidInput(_))
            ),
            "{id:?}"
        );
        assert!(
            matches!(
                dataset.download(id, None).await,
                Err(Error::InvalidInput(_))
            ),
            "{id:?}"
        );
    }
    assert!(server.received().is_empty());
}

#[tokio::test]
async fn downloads_from_storage_without_the_api_key() {
    let server = Server::start(|index, _| match index {
        0 => response(307, &[("location", "/storage/a.mmdb?sig=secret")], b""),
        1 => response(302, &[("location", "/storage/b.mmdb?sig=secret")], b""),
        _ => response(
            200,
            &[("content-type", "application/octet-stream")],
            b"mmdb bytes",
        ),
    })
    .await;

    let mut download = server
        .client()
        .geolocation()
        .download("2026-11-17", Some(ExportFormat::Mmdb))
        .await
        .unwrap();
    let mut body = Vec::new();
    while let Some(chunk) = download.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }

    assert_eq!(body, b"mmdb bytes");
    let received = server.received();
    assert_eq!(
        received[0].target,
        "/api/v4/feeds/geolocation/export/2026-11-17?format=mmdb"
    );
    assert_eq!(received[0].header("x-api-key"), Some(API_KEY));
    for storage in &received[1..] {
        assert_eq!(storage.header("x-api-key"), None, "{}", storage.target);
    }
    assert_eq!(received[2].target, "/storage/b.mmdb?sig=secret");
}
