use std::time::Duration;

use reqwest::header::{AsHeaderName, HeaderMap};
use reqwest::{Response, StatusCode};

/// Describes one HTTP response from the Synthient API. See
/// [`ClientBuilder::on_response`](crate::ClientBuilder::on_response).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResponseMetadata {
    /// The HTTP status of the response.
    pub status: StatusCode,
    /// Identifies the request when contacting Synthient support.
    pub request_id: Option<String>,
    /// The rate limit that applies to the operation, or `None` if the
    /// response does not report one, as on health checks.
    pub rate_limit: Option<RateLimit>,
    /// A copy of the response headers. Headers can contain sensitive data,
    /// such as presigned download URLs.
    pub headers: HeaderMap,
}

impl ResponseMetadata {
    pub(crate) fn new(status: StatusCode, headers: &HeaderMap) -> Self {
        Self {
            status,
            request_id: request_id(headers),
            rate_limit: RateLimit::from_headers(headers),
            headers: headers.clone(),
        }
    }
}

/// The state of an operation's rate limit, from the `RateLimit-Limit`,
/// `RateLimit-Remaining`, and `RateLimit-Reset` headers. Limits are shared
/// by every key of a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RateLimit {
    /// The bucket size, which is the burst the operation allows.
    pub limit: u64,
    /// The number of requests left in the bucket.
    pub remaining: u64,
    /// How long until the bucket is full again.
    pub reset: Duration,
}

impl RateLimit {
    fn from_headers(headers: &HeaderMap) -> Option<Self> {
        Some(Self {
            limit: header_number(headers, "ratelimit-limit")?,
            remaining: header_number(headers, "ratelimit-remaining")?,
            reset: Duration::from_secs(header_number(headers, "ratelimit-reset")?),
        })
    }
}

pub(crate) fn request_id(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("x-request-id")?.to_str().ok()?;
    Some(value.to_owned())
}

/// Parses a header holding a non-negative integer, such as a rate limit or a
/// Retry-After given in seconds.
pub(crate) fn header_number(headers: &HeaderMap, name: impl AsHeaderName) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Reads the response body into `body`, up to `limit` bytes. Returns whether
/// the body was longer. On a read error, `body` keeps what was read.
pub(crate) async fn read_body(
    response: &mut Response,
    limit: usize,
    body: &mut Vec<u8>,
) -> reqwest::Result<bool> {
    while let Some(chunk) = response.chunk().await? {
        let room = limit - body.len();
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            return Ok(true);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(name, value)| (name.parse().unwrap(), HeaderValue::from_static(value)))
            .collect()
    }

    #[test]
    fn parses_rate_limit_headers() {
        let headers = headers(&[
            ("RateLimit-Limit", "200"),
            ("RateLimit-Remaining", "199"),
            ("RateLimit-Reset", "1"),
            ("X-Request-Id", "0123abcd"),
        ]);

        let meta = ResponseMetadata::new(StatusCode::OK, &headers);

        assert_eq!(meta.request_id.as_deref(), Some("0123abcd"));
        assert_eq!(
            meta.rate_limit,
            Some(RateLimit {
                limit: 200,
                remaining: 199,
                reset: Duration::from_secs(1),
            })
        );
    }

    #[test]
    fn rate_limit_needs_every_header() {
        let partial = headers(&[("RateLimit-Limit", "200"), ("RateLimit-Remaining", "199")]);
        let invalid = headers(&[
            ("RateLimit-Limit", "200"),
            ("RateLimit-Remaining", "-1"),
            ("RateLimit-Reset", "1"),
        ]);

        assert_eq!(RateLimit::from_headers(&partial), None);
        assert_eq!(RateLimit::from_headers(&invalid), None);
    }
}
