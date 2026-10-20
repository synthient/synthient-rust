use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Response, StatusCode};

use crate::response::header_number;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A longer Retry-After is returned to the caller instead of waited out.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryPolicy {
    /// Retries rate limits, gateway and server errors, and transport
    /// failures. Used for operations that cost no credits.
    Transient,
    /// Retries only rate limits, which the API rejects before doing any work.
    /// Used for billed lookups, where a retried server error or transport
    /// failure could be charged twice, and for the health check, which should
    /// report a failure right away.
    RateLimit,
}

impl RetryPolicy {
    pub(crate) fn delay(
        self,
        result: &reqwest::Result<Response>,
        attempt: u32,
        max_retries: u32,
    ) -> Option<Duration> {
        if attempt >= max_retries {
            return None;
        }
        match result {
            Err(err) => self.transport_delay(err, attempt, max_retries),
            Ok(response) if self.retries(response) => {
                let retry_after = parse_retry_after(response.headers()).unwrap_or_default();
                (retry_after <= MAX_RETRY_AFTER).then(|| backoff(attempt).max(retry_after))
            }
            Ok(_) => None,
        }
    }

    pub(crate) fn transport_delay(
        self,
        err: &reqwest::Error,
        attempt: u32,
        max_retries: u32,
    ) -> Option<Duration> {
        (attempt < max_retries && self == Self::Transient && !err.is_builder())
            .then(|| backoff(attempt))
    }

    fn retries(self, response: &Response) -> bool {
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            // Only the rate limiter sets Retry-After.
            return response
                .headers()
                .get(RETRY_AFTER)
                .is_some_and(|value| !value.is_empty());
        }
        self == Self::Transient && matches!(status.as_u16(), 500 | 502 | 503 | 504)
    }
}

/// Returns an exponential backoff of one second doubled per attempt, capped
/// at 30 seconds, with 25% jitter either way.
fn backoff(attempt: u32) -> Duration {
    let base = Duration::from_secs(1 << attempt.min(5)).min(MAX_BACKOFF);
    // RandomState is seeded randomly, which is enough for jitter and avoids
    // a dependency on a random number generator.
    let random = RandomState::new().build_hasher().finish();
    let fraction = (random >> 11) as f64 / (1u64 << 53) as f64;
    base.mul_f64(0.75 + fraction / 2.0)
}

/// Reads only the seconds form, because the API never sends the HTTP-date
/// form.
pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    header_number(headers, RETRY_AFTER).map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    #[test]
    fn backoff_doubles_with_jitter_up_to_the_cap() {
        for (attempt, base) in [(0, 1), (1, 2), (2, 4), (4, 16), (5, 30), (40, 30)] {
            let base = Duration::from_secs(base);
            for _ in 0..100 {
                let delay = backoff(attempt);
                assert!(delay >= base.mul_f64(0.75), "attempt {attempt}: {delay:?}");
                assert!(delay <= base.mul_f64(1.25), "attempt {attempt}: {delay:?}");
            }
        }
    }

    #[test]
    fn parses_retry_after_seconds() {
        let cases = [
            ("5", Some(5)),
            (" 7 ", Some(7)),
            ("0", Some(0)),
            ("-1", None),
            ("1.5", None),
            ("Wed, 21 Oct 2015 07:28:00 GMT", None),
            ("", None),
        ];
        for (value, want) in cases {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_static(value));
            assert_eq!(
                parse_retry_after(&headers),
                want.map(Duration::from_secs),
                "{value:?}"
            );
        }
        assert_eq!(parse_retry_after(&HeaderMap::new()), None);
    }
}
