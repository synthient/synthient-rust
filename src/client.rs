use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue, USER_AGENT};
use reqwest::redirect::Policy;
use reqwest::{Method, Response, Url};
use serde::de::DeserializeOwned;
use tokio::time::Instant;

use crate::error::{Error, Result, api_error};
use crate::events::{
    AnonymizerEvent, HeliosAdbEvent, HeliosDnsEvent, HeliosHttpEvent, HeliosHttpsEvent, Ja4tEvent,
    ProxyEvent, TorrentEvent,
};
use crate::feed::Feed;
use crate::response::{ResponseMetadata, read_body};
use crate::retry::RetryPolicy;

/// The base URL of the Synthient HTTP API.
pub const DEFAULT_BASE_URL: &str = "https://api.synthient.com/api/v4";

/// The version of this crate. It is sent in the User-Agent header.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_MAX_RETRIES: u32 = 2;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RESPONSE_SIZE: usize = 16 << 20;
const JSON: &str = "application/json";
const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

type ResponseHook = Arc<dyn Fn(&ResponseMetadata) + Send + Sync>;

/// A Synthient API client.
///
/// Create one with [`Client::new`] or [`Client::builder`] and reuse it for
/// every request. Cloning is cheap, and clones share one connection pool.
/// Requests need a Tokio runtime with its time driver enabled.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    user_agent: HeaderValue,
    api_key: HeaderValue,
    base_url: Url,
    max_retries: u32,
    timeout: Option<Duration>,
    on_response: Option<ResponseHook>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url.as_str())
            .field("max_retries", &self.inner.max_retries)
            .field("timeout", &self.inner.timeout)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Returns a client that authenticates with `api_key` and uses the
    /// default configuration. It performs no network I/O.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the key is empty or has characters
    /// that cannot be sent in a header, or [`Error::Http`] if the TLS backend
    /// cannot be initialized.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::builder(api_key).build()
    }

    /// Returns a builder for a client that authenticates with `api_key`.
    pub fn builder(api_key: impl Into<String>) -> ClientBuilder {
        ClientBuilder {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: None,
            user_agent: None,
            http: None,
            on_response: None,
        }
    }

    /// The residential, mobile, and datacenter proxy feed.
    pub fn proxies(&self) -> Feed<ProxyEvent> {
        Feed::new(self.clone(), &["feeds", "proxies"])
    }

    /// The VPN and anonymizer IP range feed.
    pub fn anonymizers(&self) -> Feed<AnonymizerEvent> {
        Feed::new(self.clone(), &["feeds", "anonymizers"])
    }

    /// The BitTorrent DHT and tracker feed.
    pub fn torrents(&self) -> Feed<TorrentEvent> {
        Feed::new(self.clone(), &["feeds", "torrents"])
    }

    /// The JA4T TCP fingerprint feed.
    pub fn ja4t(&self) -> Feed<Ja4tEvent> {
        Feed::new(self.clone(), &["feeds", "ja4t"])
    }

    /// The Helios honeypot HTTP capture feed.
    pub fn helios_http(&self) -> Feed<HeliosHttpEvent> {
        Feed::new(self.clone(), &["feeds", "helio", "http"])
    }

    /// The Helios honeypot TLS ClientHello capture feed.
    pub fn helios_https(&self) -> Feed<HeliosHttpsEvent> {
        Feed::new(self.clone(), &["feeds", "helio", "https"])
    }

    /// The Helios honeypot tunnel destination feed. Despite its name, it does
    /// not carry DNS queries.
    pub fn helios_dns(&self) -> Feed<HeliosDnsEvent> {
        Feed::new(self.clone(), &["feeds", "helio", "dns"])
    }

    /// The Helios honeypot Android Debug Bridge command feed.
    pub fn helios_adb(&self) -> Feed<HeliosAdbEvent> {
        Feed::new(self.clone(), &["feeds", "helio", "adb"])
    }

    /// Returns the API URL for `segments`, each percent-encoded as one path
    /// segment under the base URL.
    pub(crate) fn endpoint<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Url {
        let mut url = self.inner.base_url.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }

    /// `handle` gets the last response, whatever its status, and is not
    /// retried. The client's timeout covers the attempts and `handle`. A retry
    /// that would wait past the timeout is skipped, so the caller gets the
    /// last response rather than [`Error::Timeout`].
    pub(crate) async fn execute<T>(
        &self,
        request: &Request,
        handle: impl AsyncFnOnce(Response) -> Result<T>,
    ) -> Result<T> {
        self.with_timeout(async |deadline| {
            let response = self
                .send(request, deadline, async |response| Ok(response))
                .await?;
            handle(response).await
        })
        .await
    }

    async fn with_timeout<T>(
        &self,
        operation: impl AsyncFnOnce(Option<Instant>) -> Result<T>,
    ) -> Result<T> {
        let deadline = self
            .inner
            .timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));
        let operation = operation(deadline);
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, operation)
                .await
                .map_err(|_| Error::Timeout)?,
            None => operation.await,
        }
    }

    pub(crate) async fn request_json<T: DeserializeOwned>(&self, request: Request) -> Result<T> {
        self.with_timeout(async |deadline| {
            self.send(&request, deadline, async |mut response| {
                if !response.status().is_success() {
                    return Err(api_error(response).await);
                }
                let mut body = Vec::new();
                if read_body(&mut response, MAX_RESPONSE_SIZE, &mut body).await? {
                    return Err(Error::Protocol(format!(
                        "response body exceeds {MAX_RESPONSE_SIZE} bytes"
                    )));
                }
                Ok(serde_json::from_slice(&body)?)
            })
            .await
        })
        .await
    }

    async fn send<T>(
        &self,
        request: &Request,
        deadline: Option<Instant>,
        mut handle: impl AsyncFnMut(Response) -> Result<T>,
    ) -> Result<T> {
        let within_deadline = |delay: &Duration| {
            deadline
                .is_none_or(|deadline| deadline.saturating_duration_since(Instant::now()) > *delay)
        };
        let mut attempt = 0;
        loop {
            let result = self.attempt(request).await;
            let delay = request
                .policy
                .delay(&result, attempt, self.inner.max_retries)
                .filter(within_deadline);
            let delay = if let Some(delay) = delay {
                drop(result);
                delay
            } else {
                let result = handle(result?).await;
                let delay = match &result {
                    Err(Error::Http(err)) => request
                        .policy
                        .transport_delay(err, attempt, self.inner.max_retries)
                        .filter(within_deadline),
                    _ => None,
                };
                let Some(delay) = delay else {
                    return result;
                };
                delay
            };
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn attempt(&self, request: &Request) -> reqwest::Result<Response> {
        let mut builder = self
            .inner
            .http
            .request(request.method.clone(), request.url.clone())
            .header(X_API_KEY, self.inner.api_key.clone())
            .header(USER_AGENT, self.inner.user_agent.clone())
            .header(ACCEPT, request.accept);
        if let Some(body) = &request.body {
            builder = builder.header(CONTENT_TYPE, JSON).body(body.clone());
        }
        let response = builder.send().await?;
        if let Some(hook) = &self.inner.on_response {
            hook(&ResponseMetadata::new(
                response.status(),
                response.headers(),
            ));
        }
        Ok(response)
    }

    /// Sends a GET to a storage host, without the API key or the response
    /// hook.
    pub(crate) async fn get_storage(&self, url: Url) -> reqwest::Result<Response> {
        self.inner
            .http
            .get(url)
            .header(USER_AGENT, self.inner.user_agent.clone())
            .send()
            .await
    }
}

pub(crate) struct Request {
    pub(crate) method: Method,
    pub(crate) url: Url,
    pub(crate) body: Option<Bytes>,
    pub(crate) accept: &'static str,
    pub(crate) policy: RetryPolicy,
}

impl Request {
    pub(crate) fn get(url: Url, policy: RetryPolicy) -> Self {
        Self {
            method: Method::GET,
            url,
            body: None,
            accept: JSON,
            policy,
        }
    }
}

/// Configures a [`Client`]. Create one with [`Client::builder`].
#[must_use]
pub struct ClientBuilder {
    api_key: String,
    base_url: String,
    max_retries: u32,
    timeout: Option<Duration>,
    user_agent: Option<String>,
    http: Option<reqwest::ClientBuilder>,
    on_response: Option<ResponseHook>,
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("base_url", &self.base_url)
            .field("max_retries", &self.max_retries)
            .field("timeout", &self.timeout)
            .field("user_agent", &self.user_agent)
            .finish_non_exhaustive()
    }
}

impl ClientBuilder {
    /// Sets the API base URL, including the version path. The default is
    /// [`DEFAULT_BASE_URL`].
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Sets how many times a failed request is retried. The default is 2,
    /// and zero disables retries. See the [crate documentation](crate#retries)
    /// for which failures are retried.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Sets a time limit for each operation, covering every attempt and the
    /// waits between them. For streams and downloads, it covers opening them
    /// but not reading their bodies. There is no limit by default.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Appends a product token, such as `my-app/1.2`, to the User-Agent
    /// header.
    pub fn user_agent(mut self, product: impl Into<String>) -> Self {
        self.user_agent = Some(product.into());
        self
    }

    /// Builds the HTTP client from `builder`, for settings such as proxies,
    /// TLS roots, and connection timeouts. The SDK turns off its redirects,
    /// so the API key never reaches the storage host, and its retries, so
    /// billed lookups are never resent. Do not set a total
    /// [`timeout`](reqwest::ClientBuilder::timeout) on it, which would also
    /// cut off streams and downloads; use a read timeout instead.
    ///
    /// By default, the SDK uses a client that gives up connecting after 30
    /// seconds and fails a read that stalls for 60 seconds.
    pub fn http_client(mut self, builder: reqwest::ClientBuilder) -> Self {
        self.http = Some(builder);
        self
    }

    /// Calls `hook` after each API response's headers arrive, including
    /// retries, pages, stream handshakes, and error responses, but not
    /// download responses from the storage host. Use it to read request IDs
    /// and rate limits.
    pub fn on_response(mut self, hook: impl Fn(&ResponseMetadata) + Send + Sync + 'static) -> Self {
        self.on_response = Some(Arc::new(hook));
        self
    }

    /// Builds the client. It performs no network I/O.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the API key is empty or has
    /// characters that cannot be sent in a header, or if the base URL or
    /// User-Agent is invalid, and [`Error::Http`] if the HTTP client cannot be
    /// built.
    pub fn build(self) -> Result<Client> {
        let api_key = self.api_key.trim();
        if api_key.is_empty() {
            return Err(Error::InvalidInput("missing API key".to_owned()));
        }
        let mut api_key = HeaderValue::from_str(api_key)
            .map_err(|_| Error::InvalidInput("API key contains invalid characters".to_owned()))?;
        api_key.set_sensitive(true);

        let base_url = parse_base_url(&self.base_url)?;

        let mut user_agent = format!("synthient-rust/{VERSION}");
        if let Some(product) = &self.user_agent {
            user_agent.push(' ');
            user_agent.push_str(product);
        }
        let user_agent = HeaderValue::from_str(&user_agent)
            .map_err(|_| Error::InvalidInput("invalid user agent".to_owned()))?;

        let http = self
            .http
            .unwrap_or_else(|| {
                reqwest::Client::builder()
                    .connect_timeout(CONNECT_TIMEOUT)
                    .read_timeout(READ_TIMEOUT)
            })
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .build()?;

        Ok(Client {
            inner: Arc::new(Inner {
                http,
                user_agent,
                api_key,
                base_url,
                max_retries: self.max_retries,
                timeout: self.timeout,
                on_response: self.on_response,
            }),
        })
    }
}

fn parse_base_url(raw: &str) -> Result<Url> {
    let invalid = || Error::InvalidInput("invalid base URL".to_owned());
    let url = Url::parse(raw.trim_end_matches('/')).map_err(|_| invalid())?;
    if !is_http_url(&url) || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid());
    }
    Ok(url)
}

pub(crate) fn is_http_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_base_urls() {
        let invalid = [
            "",
            "api.synthient.com",
            "ftp://api.synthient.com",
            "https://",
            "https://user:pass@api.synthient.com",
            "https://api.synthient.com/api/v4?x=1",
            "https://api.synthient.com/api/v4#x",
        ];
        for raw in invalid {
            assert!(parse_base_url(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn endpoint_encodes_each_segment() {
        let client = Client::builder("key")
            .base_url("https://example.com/prefix/v4/")
            .build()
            .unwrap();

        let url = client.endpoint(["lookup", "domain", "a/b?c#d%e f"]);

        assert_eq!(
            url.as_str(),
            "https://example.com/prefix/v4/lookup/domain/a%2Fb%3Fc%23d%25e%20f"
        );
    }

    #[test]
    fn rejects_a_blank_api_key() {
        for key in ["", "  ", "\t\n"] {
            let err = Client::new(key).unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{key:?}");
        }
        let err = Client::new("bad\nkey").unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn debug_never_shows_the_api_key() {
        let builder = Client::builder("secret-key");
        assert!(!format!("{builder:?}").contains("secret-key"));

        let client = builder.build().unwrap();
        assert!(!format!("{client:?}").contains("secret-key"));
        assert!(!format!("{client:#?}").contains("secret-key"));
    }
}
