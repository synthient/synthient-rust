use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::stream::{self, BoxStream};
use futures_util::{Stream, StreamExt};
use reqwest::header::LOCATION;
use reqwest::{Response, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::client::{Client, Request, is_http_url};
use crate::error::{Error, Result, api_error, require_ok};
use crate::retry::RetryPolicy;
use crate::stream::EventStream;
use crate::timestamp::Timestamp;

const NDJSON: &str = "application/x-ndjson";
const MAX_STORAGE_REDIRECTS: usize = 10;

/// A Synthient data feed. Each feed has a real-time stream of events of type
/// `T` and hourly parquet exports that roll up into daily exports. Get one
/// from a [`Client`] method such as [`Client::proxies`].
///
/// Exports are identified by an ID: `latest`, a UTC day as `YYYY-MM-DD`, or
/// a UTC hour as `YYYY-MM-DD/H`, where a zero-padded hour is also accepted.
/// [`Export::id`] values can be passed as they are. An hourly export is
/// available until its day's daily export is rolled up, shortly after 00:30
/// UTC the next day. After that, requests for the hour return a 400
/// [`Error::Api`]. While a daily export is being rolled up, requests for it
/// return a 404 [`Error::Api`] with `retry_after` set.
pub struct Feed<T> {
    client: Client,
    pub(crate) path: &'static [&'static str],
    event: PhantomData<fn() -> T>,
}

impl<T> Clone for Feed<T> {
    fn clone(&self) -> Self {
        Self::new(self.client.clone(), self.path)
    }
}

impl<T> fmt::Debug for Feed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Feed")
            .field("path", &self.path.join("/"))
            .finish_non_exhaustive()
    }
}

impl<T> Feed<T> {
    pub(crate) fn new(client: Client, path: &'static [&'static str]) -> Self {
        Self {
            client,
            path,
            event: PhantomData,
        }
    }

    /// Returns one page of the feed's exports, newest first. `limit` sets the
    /// page size, from 1 to 500, and the API default is 100. Pass a page's
    /// [`next_cursor`](ExportPage::next_cursor) as `cursor` to fetch the page
    /// after it, or call [`all_exports`](Self::all_exports) to read every
    /// export across pages.
    ///
    /// ```no_run
    /// # async fn run(client: synthient::Client) -> Result<(), synthient::Error> {
    /// let page = client.proxies().list_exports(Some(10), None).await?;
    /// for export in page.exports {
    ///     println!("{} {} rows", export.id, export.row_count);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_exports(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<ExportPage> {
        let mut url = self
            .client
            .endpoint(self.path.iter().copied().chain(["export"]));
        if let Some(limit) = limit {
            url.query_pairs_mut()
                .append_pair("limit", &limit.to_string());
        }
        if let Some(cursor) = cursor {
            url.query_pairs_mut().append_pair("cursor", cursor);
        }
        self.client
            .request_json(Request::get(url, RetryPolicy::Transient))
            .await
    }

    /// Returns a stream of every export, newest first, starting at `cursor`
    /// when it is set. Pages of `limit` exports are fetched as the stream is
    /// read.
    pub fn all_exports(&self, limit: Option<u32>, cursor: Option<&str>) -> ExportStream
    where
        T: 'static,
    {
        let pages = Pages {
            feed: self.clone(),
            limit,
            cursor: cursor.map(str::to_owned),
            done: false,
            seen: HashSet::new(),
            exports: Vec::new().into_iter(),
        };
        let inner = stream::try_unfold(pages, |mut pages| async move {
            let export = pages.next().await?;
            Ok(export.map(|export| (export, pages)))
        });
        EventStream::from_stream(inner.boxed())
    }

    /// Returns the checksum, size, row count, and parquet schema of an
    /// export.
    pub async fn export_meta(&self, id: &str) -> Result<ExportMeta> {
        let url = self.export_url(id, &["meta"])?;
        self.client
            .request_json(Request::get(url, RetryPolicy::Transient))
            .await
    }

    /// Returns a presigned URL for an export's parquet file, for tools that
    /// read parquet over HTTPS directly. The URL is valid for 24 hours and
    /// grants access without an API key, so treat it as a secret.
    pub async fn download_url(&self, id: &str) -> Result<Url> {
        let request = Request::get(self.export_url(id, &[])?, RetryPolicy::Transient);
        self.client
            .execute(&request, async |response| {
                export_location(&request.url, response).await
            })
            .await
    }

    /// Opens an export's parquet file for reading. The API key is never sent
    /// to the storage host that serves the file, and errors never include the
    /// presigned URL.
    ///
    /// # Errors
    ///
    /// A non-success response from the API or the storage host returns an
    /// [`Error::Api`].
    pub async fn download(&self, id: &str) -> Result<Download> {
        let request = Request::get(self.export_url(id, &[])?, RetryPolicy::Transient);
        self.client
            .execute(&request, async |response| {
                let mut url = export_location(&request.url, response).await?;
                for _ in 0..=MAX_STORAGE_REDIRECTS {
                    let response = self.client.get_storage(url.clone()).await?;
                    if response.status().is_redirection() {
                        url = redirect_target(&url, &response)?;
                        continue;
                    }
                    return require_ok(response).await.map(Download::new);
                }
                Err(Error::Protocol("too many export redirects".to_owned()))
            })
            .await
    }

    /// Returns the API URL of an export, followed by `tail`. Only digits are
    /// accepted in the ID, so it cannot change the path.
    fn export_url(&self, id: &str, tail: &[&str]) -> Result<Url> {
        let export = export_segments(id)?;
        let segments = self.path.iter().copied().chain(["export"]);
        Ok(self.client.endpoint(
            segments
                .chain(export.iter().map(String::as_str))
                .chain(tail.iter().copied()),
        ))
    }
}

impl<T: DeserializeOwned + Send + 'static> Feed<T> {
    /// Opens the feed's real-time event stream. Each call opens a new
    /// connection, which the server closes after up to 30 minutes. Events
    /// sent between connections are missed, so call `stream` again to
    /// reconnect.
    ///
    /// ```no_run
    /// # async fn run(client: synthient::Client) -> Result<(), synthient::Error> {
    /// let mut events = client.proxies().stream().await?;
    /// while let Some(event) = events.next().await {
    ///     let event = event?;
    ///     println!("{:?} {:?}", event.ip, event.provider);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// A non-success response returns an [`Error::Api`].
    pub async fn stream(&self) -> Result<EventStream<T>> {
        let url = self
            .client
            .endpoint(self.path.iter().copied().chain(["stream"]));
        let request = Request {
            accept: NDJSON,
            ..Request::get(url, RetryPolicy::Transient)
        };
        let response = self.client.execute(&request, require_ok).await?;
        Ok(EventStream::new(response))
    }
}

/// Splits an export ID into its path segments, normalizing a zero-padded
/// hour.
fn export_segments(id: &str) -> Result<Vec<String>> {
    let invalid = || Error::InvalidInput(format!("invalid export ID {id:?}"));
    if id == "latest" {
        return Ok(vec![id.to_owned()]);
    }

    let (date, hour) = match id.split_once('/') {
        Some((date, hour)) => (date, Some(hour)),
        None => (id, None),
    };
    let mut parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    let valid_date = digits(year, 4).is_some()
        && digits(month, 2).is_some_and(|month| (1..=12).contains(&month))
        && digits(day, 2).is_some_and(|day| (1..=31).contains(&day));
    if !valid_date {
        return Err(invalid());
    }

    let Some(hour) = hour else {
        return Ok(vec![date.to_owned()]);
    };
    let hour = digits(hour, 1)
        .or_else(|| digits(hour, 2))
        .filter(|hour| *hour <= 23)
        .ok_or_else(invalid)?;
    Ok(vec![date.to_owned(), hour.to_string()])
}

/// Parses `text` if it is exactly `len` ASCII digits.
fn digits(text: &str, len: usize) -> Option<u32> {
    if text.len() != len || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Returns the storage URL the API redirects an export request to.
async fn export_location(from: &Url, response: Response) -> Result<Url> {
    if response.status().is_client_error() || response.status().is_server_error() {
        return Err(api_error(response).await);
    }
    if !response.status().is_redirection() {
        return Err(Error::Protocol(format!(
            "expected a redirect to the export, got {}",
            response.status()
        )));
    }
    redirect_target(from, &response)
}

/// Resolves a redirect's Location. Only http(s) targets without credentials
/// are allowed, and an https request never redirects to http.
fn redirect_target(from: &Url, response: &Response) -> Result<Url> {
    let invalid = || Error::Protocol("export redirect has no valid location".to_owned());
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(invalid)?;
    let target = from.join(location.trim()).map_err(|_| invalid())?;
    if !is_http_url(&target) {
        return Err(invalid());
    }
    if from.scheme() == "https" && target.scheme() != "https" {
        return Err(Error::Protocol(
            "export redirect does not use https".to_owned(),
        ));
    }
    Ok(target)
}

/// One page of a feed's exports, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportPage {
    /// The feed's stream identifier, such as `proxies`.
    pub stream: String,
    /// The exports on this page. The API names the field `feeds`.
    #[serde(rename = "feeds")]
    pub exports: Vec<Export>,
    /// The cursor of the next page, for [`Feed::list_exports`] or
    /// [`Feed::all_exports`]. It is `None` on the last page.
    #[serde(default, deserialize_with = "non_empty")]
    pub next_cursor: Option<String>,
}

/// Decodes an optional string, reading an empty one as `None`. The API ends
/// export listings with either a null or an empty cursor.
fn non_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.filter(|value| !value.is_empty()))
}

/// A parquet export of a feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Export {
    /// Identifies the export for [`Feed::export_meta`], [`Feed::download`],
    /// and [`Feed::download_url`]. On the first page, the newest hourly
    /// export may have the ID `latest`.
    pub id: String,
    /// Whether the export covers an hour or a day.
    pub kind: ExportKind,
    /// The UTC day of the export as `YYYY-MM-DD`.
    pub date: String,
    /// The UTC hour of an hourly export, or `None` for a daily export.
    pub hour: Option<u8>,
    /// The size of the parquet file in bytes.
    pub size_bytes: i64,
    /// The number of rows in the parquet file.
    pub row_count: i64,
    /// The hex-encoded SHA-256 of the parquet file.
    pub checksum: String,
    /// When the export was produced.
    pub created_at: Timestamp,
    /// The API path of the export's download, including the version prefix.
    pub download_path: String,
}

wire_enum! {
    /// Whether an export covers an hour or a day.
    pub enum ExportKind {
        /// One UTC hour.
        Hourly = "hourly",
        /// One UTC day.
        Daily = "daily",
    }
}

/// A parquet export's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportMeta {
    /// Identifies the export.
    pub id: String,
    /// The feed's stream identifier.
    pub stream: String,
    /// Whether the export covers an hour or a day.
    pub kind: ExportKind,
    /// The file format, `parquet`.
    pub format: String,
    /// The start of the export's UTC day or hour.
    pub date: Timestamp,
    /// The UTC hour of an hourly export, or `None` for a daily export.
    pub hour: Option<u8>,
    /// The number of rows in the parquet file.
    pub rows: i64,
    /// The size of the parquet file in bytes.
    pub size: i64,
    /// The hex-encoded SHA-256 of the parquet file.
    pub checksum: String,
    /// When the export was produced.
    pub created_at: Timestamp,
    /// The parquet columns.
    pub schema: ParquetSchema,
}

/// The columns of a parquet export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ParquetSchema {
    /// The columns in file order.
    pub fields: Vec<ParquetColumn>,
}

/// A column of a parquet export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ParquetColumn {
    /// The column name.
    pub name: String,
    /// The column type.
    pub r#type: ParquetType,
}

wire_enum! {
    /// The type of a parquet column.
    pub enum ParquetType {
        /// A UTF-8 string.
        String = "string",
        /// A signed 64-bit integer.
        Int64 = "int64",
        /// An unsigned 32-bit integer.
        Uint32 = "uint32",
        /// An unsigned 64-bit integer.
        Uint64 = "uint64",
        /// A boolean.
        Bool = "bool",
        /// A byte array.
        Bytes = "bytes",
    }
}

struct Pages<T> {
    feed: Feed<T>,
    limit: Option<u32>,
    cursor: Option<String>,
    /// Whether the last page has been fetched.
    done: bool,
    seen: HashSet<String>,
    exports: std::vec::IntoIter<Export>,
}

impl<T> Pages<T> {
    async fn next(&mut self) -> Result<Option<Export>> {
        loop {
            if let Some(export) = self.exports.next() {
                return Ok(Some(export));
            }
            if self.done {
                return Ok(None);
            }
            if let Some(cursor) = &self.cursor {
                if !self.seen.insert(cursor.clone()) {
                    return Err(Error::Protocol(
                        "export listing returned a repeated cursor".to_owned(),
                    ));
                }
            }
            let page = self
                .feed
                .list_exports(self.limit, self.cursor.as_deref())
                .await?;
            self.exports = page.exports.into_iter();
            self.done = page.next_cursor.is_none();
            self.cursor = page.next_cursor;
        }
    }
}

/// A stream of every export of a feed, newest first. Create one with
/// [`Feed::all_exports`].
pub type ExportStream = EventStream<Export>;

/// The contents of an export's parquet file, read in chunks as they arrive.
/// Create one with [`Feed::download`]. Dropping it closes the connection.
pub struct Download {
    inner: BoxStream<'static, Result<Bytes>>,
    content_length: Option<u64>,
}

impl Download {
    fn new(response: Response) -> Self {
        let content_length = response.content_length();
        let inner = stream::try_unfold(response, |mut response| async move {
            let chunk = response.chunk().await?;
            Ok(chunk.map(|chunk| (chunk, response)))
        });
        Self {
            inner: inner.boxed(),
            content_length,
        }
    }

    /// Returns the size of the file in bytes, if the storage host sent it.
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// Returns the next chunk of the file, or `None` once it has been read.
    pub async fn next(&mut self) -> Option<Result<Bytes>> {
        self.inner.next().await
    }
}

impl Stream for Download {
    type Item = Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

impl fmt::Debug for Download {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Download")
            .field("content_length", &self.content_length)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_ids_become_path_segments() {
        let cases: [(&str, &[&str]); 5] = [
            ("latest", &["latest"]),
            ("2026-05-03", &["2026-05-03"]),
            ("2026-05-03/7", &["2026-05-03", "7"]),
            ("2026-05-03/07", &["2026-05-03", "7"]),
            ("2026-05-03/23", &["2026-05-03", "23"]),
        ];
        for (id, want) in cases {
            assert_eq!(export_segments(id).unwrap(), want, "{id:?}");
        }
    }

    #[test]
    fn rejects_invalid_export_ids() {
        let invalid = [
            "",
            "LATEST",
            "latest/1",
            "2026-5-03",
            "2026-13-01",
            "2026-00-01",
            "2026-01-32",
            "2026-01-01/24",
            "2026-01-01/007",
            "2026-01-01/",
            "2026-01-01/+1",
            "2026-01-01/1/meta",
            "2026-01-01-01",
            "../2026-01-01",
            "２０２６-01-01",
        ];
        for id in invalid {
            assert!(
                matches!(export_segments(id), Err(Error::InvalidInput(_))),
                "{id:?}"
            );
        }
    }
}
