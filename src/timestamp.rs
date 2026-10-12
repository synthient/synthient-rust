use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// As seconds, values this large would be tens of thousands of years away.
const MILLISECOND_THRESHOLD: i64 = 1_000_000_000_000;

/// A Unix time exactly as the API encodes it.
///
/// It is in seconds, except on anonymizer and Helios HTTP, HTTPS, and DNS
/// stream events, where it is in milliseconds.
/// [`to_system_time`](Self::to_system_time) converts either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Returns the time as a [`SystemTime`], which converts into the time
    /// types of crates such as `chrono`, `time`, and `jiff`. Values of 1e12
    /// or more are read as milliseconds. Returns `None` if the time is out of
    /// the platform's range.
    pub fn to_system_time(self) -> Option<SystemTime> {
        let offset = if self.0 >= MILLISECOND_THRESHOLD {
            Duration::from_millis(self.0.unsigned_abs())
        } else {
            Duration::from_secs(self.0.unsigned_abs())
        };
        if self.0 >= 0 {
            UNIX_EPOCH.checked_add(offset)
        } else {
            UNIX_EPOCH.checked_sub(offset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_seconds_and_milliseconds() {
        let cases = [
            (0, Duration::ZERO),
            (1_778_200_137, Duration::from_secs(1_778_200_137)),
            (999_999_999_999, Duration::from_secs(999_999_999_999)),
            (1_778_200_137_487, Duration::from_millis(1_778_200_137_487)),
        ];
        for (raw, since_epoch) in cases {
            assert_eq!(
                Timestamp(raw).to_system_time(),
                Some(UNIX_EPOCH + since_epoch),
                "{raw}"
            );
        }
        assert_eq!(
            Timestamp(-60).to_system_time(),
            Some(UNIX_EPOCH - Duration::from_secs(60))
        );
    }

    #[test]
    fn extreme_values_do_not_panic() {
        let _ = Timestamp(i64::MAX).to_system_time();
        let _ = Timestamp(i64::MIN).to_system_time();
    }

    #[test]
    fn round_trips_the_wire_value() {
        let timestamp: Timestamp = serde_json::from_str("1778200137487").unwrap();

        assert_eq!(timestamp, Timestamp(1_778_200_137_487));
        assert_eq!(serde_json::to_string(&timestamp).unwrap(), "1778200137487");
    }
}
