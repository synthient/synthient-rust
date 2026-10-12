use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures_util::stream::{self, BoxStream};
use futures_util::{Stream, StreamExt};
use reqwest::Response;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// The longest stream line buffered while waiting for its newline.
const MAX_EVENT_SIZE: usize = 16 << 20;

/// A real-time stream of feed events. Create one with
/// [`Feed::stream`](crate::Feed::stream).
///
/// The server closes each stream cleanly after about 30 minutes, which ends
/// it. Any other failure, such as a dropped connection or an undecodable
/// event, is returned once, and the stream then ends. Dropping the stream
/// closes its connection.
///
/// Read events with [`next`](Self::next), or use the [`Stream`] trait with
/// combinators from `futures` or `tokio-stream`.
pub struct EventStream<T> {
    inner: BoxStream<'static, Result<T>>,
}

impl<T: DeserializeOwned + Send + 'static> EventStream<T> {
    pub(crate) fn new(response: Response) -> Self {
        let state = (response, Lines::default());
        let inner = stream::try_unfold(state, |(mut response, mut lines)| async move {
            let event = next_event(&mut response, &mut lines).await?;
            Ok(event.map(|event| (event, (response, lines))))
        });
        Self {
            inner: inner.boxed(),
        }
    }
}

impl<T> EventStream<T> {
    /// Returns the next event, or `None` once the stream has ended.
    pub async fn next(&mut self) -> Option<Result<T>> {
        self.inner.next().await
    }
}

impl<T> Stream for EventStream<T> {
    type Item = Result<T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

impl<T> fmt::Debug for EventStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventStream").finish_non_exhaustive()
    }
}

/// Reads the next event, skipping blank keepalive lines. A final line without
/// a newline is still decoded, so a truncated event is an error rather than
/// a silent end.
async fn next_event<T: DeserializeOwned>(
    response: &mut Response,
    lines: &mut Lines,
) -> Result<Option<T>> {
    loop {
        if let Some(line) = lines.next_line() {
            if line.trim_ascii().is_empty() {
                continue;
            }
            return Ok(Some(serde_json::from_slice(&line)?));
        }
        if lines.buffer.len() > MAX_EVENT_SIZE {
            return Err(Error::Protocol(format!(
                "stream event exceeds {MAX_EVENT_SIZE} bytes"
            )));
        }
        let Some(chunk) = response.chunk().await? else {
            let rest = lines.take_rest();
            if rest.trim_ascii().is_empty() {
                return Ok(None);
            }
            return Ok(Some(serde_json::from_slice(&rest)?));
        };
        lines.buffer.extend_from_slice(&chunk);
    }
}

/// Splits bytes that arrive in arbitrary chunks into newline-terminated
/// lines.
#[derive(Default)]
struct Lines {
    buffer: BytesMut,
    /// How much of the buffer is known to hold no newline.
    searched: usize,
}

impl Lines {
    /// Returns the next complete line, including its newline.
    fn next_line(&mut self) -> Option<BytesMut> {
        let Some(offset) = memchr::memchr(b'\n', &self.buffer[self.searched..]) else {
            self.searched = self.buffer.len();
            return None;
        };
        let line = self.buffer.split_to(self.searched + offset + 1);
        self.searched = 0;
        Some(line)
    }

    fn take_rest(&mut self) -> BytesMut {
        self.searched = 0;
        self.buffer.split()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_lines(chunks: &[&[u8]]) -> Vec<String> {
        let mut lines = Lines::default();
        let mut out = Vec::new();
        for chunk in chunks {
            lines.buffer.extend_from_slice(chunk);
            while let Some(line) = lines.next_line() {
                out.push(String::from_utf8(line.to_vec()).unwrap());
            }
        }
        let rest = lines.take_rest();
        if !rest.is_empty() {
            out.push(String::from_utf8(rest.to_vec()).unwrap());
        }
        out
    }

    #[test]
    fn splits_lines_at_every_chunk_boundary() {
        let input = "{\"a\":1}\n\n{\"b\":\"é\"}\r\n{\"c\":3}".as_bytes();
        let want = ["{\"a\":1}\n", "\n", "{\"b\":\"é\"}\r\n", "{\"c\":3}"];

        for first in 0..=input.len() {
            for second in first..=input.len() {
                let chunks = [&input[..first], &input[first..second], &input[second..]];
                assert_eq!(split_lines(&chunks), want, "split at {first} and {second}");
            }
        }
    }

    #[test]
    fn one_byte_chunks() {
        let input = b"{\"a\":1}\n{\"a\":2}\n";
        let chunks: Vec<&[u8]> = input.chunks(1).collect();

        assert_eq!(split_lines(&chunks), ["{\"a\":1}\n", "{\"a\":2}\n"]);
    }

    #[test]
    fn keeps_the_unterminated_line() {
        let mut lines = Lines::default();
        lines.buffer.extend_from_slice(b"{\"a\":1}\n{\"b\"");

        assert!(lines.next_line().is_some());
        assert!(lines.next_line().is_none());
        assert_eq!(&lines.buffer[..], b"{\"b\"");
    }
}
