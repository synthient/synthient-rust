use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Response, StatusCode};
use serde::Deserialize;

use crate::response::{read_body, request_id};
use crate::retry::parse_retry_after;

/// The largest error body read for diagnostics. Longer bodies are cut off.
const MAX_ERROR_SIZE: usize = 64 << 10;

/// A `Result` whose error is [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error from the Synthient client.
///
/// Match on [`Error::Api`] to handle a specific HTTP status:
///
/// ```no_run
/// # async fn run(client: synthient::Client) {
/// use synthient::{Error, StatusCode};
///
/// match client.account().await {
///     Ok(account) => println!("{} credits left", account.lookup_quota.credits),
///     Err(Error::Api(err)) if err.status == StatusCode::PAYMENT_REQUIRED => {
///         eprintln!("out of lookup credits");
///     }
///     Err(err) => eprintln!("{err}"),
/// }
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An argument or configuration value was rejected before anything was
    /// sent.
    InvalidInput(String),
    /// The API, or the storage host serving a download, answered with a
    /// non-success status.
    Api(Box<ApiError>),
    /// The HTTP client could not be built, the request could not be sent, or
    /// the response could not be read. URLs are removed from these errors
    /// because download URLs grant access to an export.
    Http(reqwest::Error),
    /// The operation took longer than the client's timeout.
    Timeout,
    /// A response or stream event was not the JSON the API documents.
    Decode(serde_json::Error),
    /// A response broke the API's contract, such as an oversized body, a
    /// missing redirect, or a repeated pagination cursor.
    Protocol(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Protocol(message) => f.write_str(message),
            Self::Api(err) => err.fmt(f),
            Self::Http(_) => f.write_str("http request failed"),
            Self::Timeout => f.write_str("operation timed out"),
            Self::Decode(_) => f.write_str("failed to decode response"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(err) => Some(err),
            Self::Decode(err) => Some(err),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.without_url())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Decode(err)
    }
}

/// A non-success HTTP response.
///
/// Its `Display` output contains only the status, so it is safe to log. The
/// other fields hold untrusted server data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApiError {
    /// The HTTP status of the response.
    pub status: StatusCode,
    /// A URI identifying the problem type, such as
    /// `https://docs.synthient.com/errors/unauthorized`.
    pub problem_type: Option<String>,
    /// A short summary of the problem type.
    pub title: Option<String>,
    /// An explanation of this occurrence of the problem. For a plain-text
    /// error, such as the edge's 401 for a malformed key, it is the response
    /// body.
    pub detail: Option<String>,
    /// Validation messages keyed by request field.
    pub errors: BTreeMap<String, Vec<String>>,
    /// Identifies the request when contacting Synthient support.
    pub request_id: Option<String>,
    /// How long the server asked the client to wait before retrying.
    pub retry_after: Option<Duration>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "api returned {}", self.status)
    }
}

impl std::error::Error for ApiError {}

#[derive(Default, Deserialize)]
pub(crate) struct Problem {
    #[serde(rename = "type")]
    problem_type: Option<String>,
    title: Option<String>,
    detail: Option<String>,
    instance: Option<String>,
    errors: Option<serde_json::Value>,
}

/// Reads a non-success response into an [`Error::Api`]. The status and
/// headers are kept even when the body is missing, malformed, or unreadable.
pub(crate) async fn api_error(mut response: Response) -> Error {
    let status = response.status();
    let headers = response.headers();
    let request_id = request_id(headers);
    let retry_after = parse_retry_after(headers);
    let is_text = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim_start().starts_with("text/plain"));

    let mut body = Vec::new();
    let _ = read_body(&mut response, MAX_ERROR_SIZE, &mut body).await;

    let problem = serde_json::from_slice(&body).unwrap_or_else(|_| Problem {
        detail: is_text.then(|| String::from_utf8_lossy(&body).trim().to_owned()),
        ..Problem::default()
    });
    Error::Api(Box::new(ApiError {
        status,
        problem_type: problem.problem_type,
        title: problem.title,
        detail: problem.detail,
        errors: problem
            .errors
            .and_then(|errors| serde_json::from_value(errors).ok())
            .unwrap_or_default(),
        request_id: request_id.or(problem.instance),
        retry_after,
    }))
}

/// Returns the response if its status is 200, or reads it into an
/// [`Error::Api`].
pub(crate) async fn require_ok(response: Response) -> Result<Response> {
    if response.status() != StatusCode::OK {
        return Err(api_error(response).await);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_omits_server_details() {
        let err = Error::Api(Box::new(ApiError {
            status: StatusCode::FORBIDDEN,
            problem_type: Some("https://docs.synthient.com/errors/forbidden".into()),
            title: Some("Forbidden".into()),
            detail: Some("secret detail".into()),
            errors: BTreeMap::new(),
            request_id: Some("abc".into()),
            retry_after: None,
        }));

        assert_eq!(err.to_string(), "api returned 403 Forbidden");
    }

    #[test]
    fn decode_errors_keep_their_source() {
        let source = serde_json::from_str::<u8>("x").unwrap_err();
        let err = Error::from(source);

        assert_eq!(err.to_string(), "failed to decode response");
        assert!(std::error::Error::source(&err).is_some());
    }
}
