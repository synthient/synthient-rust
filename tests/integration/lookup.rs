use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use synthient::{Cadence, DomainStatus, Error, NetworkType, Os, Relation, Scope, StatusCode};

use crate::server::{ACCOUNT, DOMAIN_INFO, IP_INFO, Server, expect_api_error, json, problem};

#[tokio::test]
async fn looks_up_an_ip() {
    let server = Server::start(|_, _| json(200, IP_INFO)).await;

    let info = server
        .client()
        .lookup_ip(Ipv4Addr::new(8, 8, 8, 8))
        .await
        .unwrap();

    assert_eq!(server.received()[0].target, "/api/v4/lookup/ip/8.8.8.8");
    assert_eq!(info.ip, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
    let network = info.network.unwrap();
    assert_eq!(network.asn, 15169);
    assert_eq!(network.r#type, NetworkType::Datacenter);
    assert_eq!(network.abuse_email, None);
    let intelligence = info.intelligence.unwrap();
    assert_eq!(intelligence.risk_score, 12);
    assert_eq!(intelligence.providers[0].name, "BRIGHTDATA");
    assert_eq!(intelligence.devices[0].os, Os::Linux);
}

#[tokio::test]
async fn looks_up_an_ipv6_address() {
    let server = Server::start(|_, _| json(200, r#"{"ip":"2001:db8::1"}"#)).await;
    let ip: Ipv6Addr = "2001:db8::1".parse().unwrap();

    let info = server.client().lookup_ip(ip).await.unwrap();

    assert_eq!(server.received()[0].target, "/api/v4/lookup/ip/2001:db8::1");
    assert_eq!(info.ip, IpAddr::V6(ip));
    assert_eq!(info.network, None);
    assert_eq!(info.intelligence, None);
}

#[tokio::test]
async fn looks_up_a_batch() {
    let body = format!(r#"{{"results":[{IP_INFO}]}}"#);
    let server = Server::start(move |_, _| json(200, &body)).await;
    let ips = ["8.8.8.8".parse().unwrap(), "2001:db8::1".parse().unwrap()];

    let results = server.client().lookup_ips(&ips).await.unwrap();

    let request = &server.received()[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/api/v4/lookup/ips");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.body, br#"{"ips":["8.8.8.8","2001:db8::1"]}"#);
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn batch_limits_are_checked_without_a_request() {
    let server = Server::start(|_, _| json(200, r#"{"results":[]}"#)).await;
    let client = server.client();

    assert!(client.lookup_ips(&[]).await.unwrap().is_empty());
    let too_many = vec![IpAddr::V4(Ipv4Addr::LOCALHOST); 1001];
    assert!(matches!(
        client.lookup_ips(&too_many).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(server.received().is_empty());

    let most = vec![IpAddr::V4(Ipv4Addr::LOCALHOST); 1000];
    client.lookup_ips(&most).await.unwrap();
    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn looks_up_a_domain() {
    let server = Server::start(|_, _| json(200, DOMAIN_INFO)).await;

    let info = server
        .client()
        .lookup_domain("example.com")
        .limit(5)
        .await
        .unwrap();

    assert_eq!(
        server.received()[0].target,
        "/api/v4/lookup/domain/example.com?limit=5"
    );
    assert_eq!(info.status, DomainStatus::Active);
    assert_eq!(info.activity_stats.cadence, Cadence::Weekend);
    assert_eq!(info.top_asn.unwrap().asn, 13335);
    assert_eq!(info.recent_events[0].source_ip, "200.13.201.29");
    assert_eq!(info.hour_dow_heatmap, [[0, 1], [2, 3]]);
}

#[tokio::test]
async fn domain_is_one_encoded_path_segment() {
    let server = Server::start(|_, _| json(200, DOMAIN_INFO)).await;

    server
        .client()
        .lookup_domain("a/../b?c#d%e é")
        .await
        .unwrap();

    assert_eq!(
        server.received()[0].target,
        "/api/v4/lookup/domain/a%2F..%2Fb%3Fc%23d%25e%20%C3%A9"
    );
}

#[tokio::test]
async fn rejects_domains_that_change_the_path() {
    let server = Server::start(|_, _| json(200, DOMAIN_INFO)).await;
    let client = server.client();

    for domain in ["", ".", ".."] {
        let result = client.lookup_domain(domain).await;
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{domain:?}");
    }
    assert!(server.received().is_empty());
}

#[tokio::test]
async fn reads_the_account() {
    let server = Server::start(|_, _| json(200, ACCOUNT)).await;

    let account = server.client().account().await.unwrap();

    assert_eq!(account.organization.relation, Relation::Owner);
    assert_eq!(
        account.scopes,
        [
            Scope::Basic,
            Scope::ProxyFeeds,
            Scope::Unknown("NEW_SCOPE".into())
        ]
    );
    assert_eq!(account.lookup_quota.credits, 1000);
    assert_eq!(
        account.api_key.unwrap().created_at,
        "2026-09-28T19:04:52.463869Z"
    );
}

#[tokio::test]
async fn unhealthy_api_is_an_error_and_not_retried() {
    let server = Server::start(|_, _| json(503, r#"{"status":"unhealthy"}"#)).await;

    let err = expect_api_error(server.client().health().await);

    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn private_addresses_return_the_validation_error() {
    let body = r#"{"type":"https://docs.synthient.com/errors/validation-failed","title":"One or more validation errors occurred.","status":422,"errors":{"ip_address":["Private address."]},"instance":"abc"}"#;
    let server = Server::start(move |_, _| problem(422, body)).await;

    let err = expect_api_error(server.client().lookup_ip(Ipv4Addr::new(10, 0, 0, 1)).await);

    assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err.errors["ip_address"], ["Private address."]);
}
