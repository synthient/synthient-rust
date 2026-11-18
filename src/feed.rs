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
    pub(crate) exports: Exports,
    event: PhantomData<fn() -> T>,
}

impl<T> Clone for Feed<T> {
    fn clone(&self) -> Self {
        Self {
            exports: self.exports.clone(),
            event: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Feed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Feed")
            .field("path", &self.exports.path.join("/"))
            .field("variant", &self.exports.variant)
            .finish_non_exhaustive()
    }
}

impl<T> Feed<T> {
    pub(crate) fn new(client: Client, path: &'static [&'static str]) -> Self {
        Self {
            exports: Exports::new(client, path, true),
            event: PhantomData,
        }
    }

    /// Returns a copy of the feed whose [`list_exports`](Self::list_exports),
    /// [`all_exports`](Self::all_exports), [`export_meta`](Self::export_meta),
    /// [`download_url`](Self::download_url), and [`download`](Self::download)
    /// ask for `variant`. Only proxies, anonymizers, and JA4T publish
    /// [`ExportVariant::Lite`] files. Other feeds list no lite exports and
    /// answer 404 for them.
    ///
    /// A lite export's ID is the same as the full one's, so download it
    /// through the lite feed:
    ///
    /// ```no_run
    /// # async fn run(client: synthient::Client) -> Result<(), synthient::Error> {
    /// use synthient::ExportVariant;
    ///
    /// let lite = client.proxies().with_variant(ExportVariant::Lite);
    /// let page = lite.list_exports(Some(1), None).await?;
    /// if let Some(export) = page.exports.first() {
    ///     let download = lite.download(&export.id).await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A lite file is published shortly after its snapshot. Until then,
    /// requests for it return a 404 [`Error::Api`].
    pub fn with_variant(&self, variant: ExportVariant) -> Self {
        Self {
            exports: Exports {
                variant: Some(variant),
                ..self.exports.clone()
            },
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
        self.exports.list(limit, cursor, None).await
    }

    /// Like [`list_exports`](Self::list_exports), but lists only the exports
    /// of `day`, a UTC day as `YYYY-MM-DD`. A day without exports returns an
    /// empty page.
    pub async fn list_exports_on(
        &self,
        day: &str,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<ExportPage> {
        self.exports.list(limit, cursor, Some(day)).await
    }

    /// Returns a stream of every export, newest first, starting at `cursor`
    /// when it is set. Pages of `limit` exports are fetched as the stream is
    /// read.
    pub fn all_exports(&self, limit: Option<u32>, cursor: Option<&str>) -> ExportStream {
        self.exports.all(limit, cursor, None)
    }

    /// Like [`all_exports`](Self::all_exports), but streams only the exports
    /// of `day`, a UTC day as `YYYY-MM-DD`.
    pub fn all_exports_on(
        &self,
        day: &str,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> ExportStream {
        self.exports.all(limit, cursor, Some(day))
    }

    /// Returns the rows and bytes exported per UTC day over the last `days`
    /// days, today included. The API default is 90 and the maximum is 366.
    pub async fn export_volume(&self, days: Option<u32>) -> Result<ExportVolume> {
        self.exports.volume(days).await
    }

    /// Returns the checksum, size, row count, and parquet schema of an
    /// export.
    pub async fn export_meta(&self, id: &str) -> Result<ExportMeta> {
        self.exports.meta(id, None).await
    }

    /// Returns a presigned URL for an export's parquet file, for tools that
    /// read parquet over HTTPS directly. The URL is valid for 24 hours and
    /// grants access without an API key, so treat it as a secret.
    pub async fn download_url(&self, id: &str) -> Result<Url> {
        self.exports.download_url(id, None).await
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
        self.exports.download(id, None).await
    }
}

impl<T: DeserializeOwned + Send + 'static> Feed<T> {
    /// Opens the feed's real-time event stream. Each call opens a new
    /// connection, which the server can close at any time, such as during a
    /// deploy. Events sent between connections are missed, so call `stream`
    /// again to reconnect.
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
        let client = &self.exports.client;
        let url = client.endpoint(self.exports.path.iter().copied().chain(["stream"]));
        let request = Request {
            accept: NDJSON,
            ..Request::get(url, RetryPolicy::Transient)
        };
        let response = client.execute(&request, require_ok).await?;
        Ok(EventStream::new(response))
    }
}

/// The export routes that feeds and datasets share.
#[derive(Clone)]
pub(crate) struct Exports {
    client: Client,
    pub(crate) path: &'static [&'static str],
    /// Datasets publish only daily exports.
    hourly: bool,
    variant: Option<ExportVariant>,
}

impl Exports {
    pub(crate) fn new(client: Client, path: &'static [&'static str], hourly: bool) -> Self {
        Self {
            client,
            path,
            hourly,
            variant: None,
        }
    }

    pub(crate) async fn list(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
        day: Option<&str>,
    ) -> Result<ExportPage> {
        let mut url = self
            .client
            .endpoint(self.path.iter().copied().chain(["export"]));
        let limit = limit.map(|limit| limit.to_string());
        append_query(
            &mut url,
            &[
                ("limit", limit.as_deref()),
                ("cursor", cursor),
                ("day", day),
                ("variant", self.variant.as_ref().map(ExportVariant::as_str)),
            ],
        );
        self.client
            .request_json(Request::get(url, RetryPolicy::Transient))
            .await
    }

    pub(crate) fn all(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
        day: Option<&str>,
    ) -> ExportStream {
        let pages = Pages {
            exports: self.clone(),
            limit,
            day: day.map(str::to_owned),
            cursor: cursor.map(str::to_owned),
            done: false,
            seen: HashSet::new(),
            items: Vec::new().into_iter(),
        };
        let inner = stream::try_unfold(pages, |mut pages| async move {
            let export = pages.next().await?;
            Ok(export.map(|export| (export, pages)))
        });
        EventStream::from_stream(inner.boxed())
    }

    pub(crate) async fn volume(&self, days: Option<u32>) -> Result<ExportVolume> {
        let mut url = self
            .client
            .endpoint(self.path.iter().copied().chain(["export", "daily"]));
        let days = days.map(|days| days.to_string());
        append_query(&mut url, &[("days", days.as_deref())]);
        self.client
            .request_json(Request::get(url, RetryPolicy::Transient))
            .await
    }

    pub(crate) async fn meta(&self, id: &str, format: Option<&ExportFormat>) -> Result<ExportMeta> {
        let url = self.export_url(id, &["meta"], format)?;
        self.client
            .request_json(Request::get(url, RetryPolicy::Transient))
            .await
    }

    pub(crate) async fn download_url(
        &self,
        id: &str,
        format: Option<&ExportFormat>,
    ) -> Result<Url> {
        let request = Request::get(self.export_url(id, &[], format)?, RetryPolicy::Transient);
        self.client
            .execute(&request, async |response| {
                export_location(&request.url, response).await
            })
            .await
    }

    pub(crate) async fn download(
        &self,
        id: &str,
        format: Option<&ExportFormat>,
    ) -> Result<Download> {
        let request = Request::get(self.export_url(id, &[], format)?, RetryPolicy::Transient);
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

    /// The ID is validated, so it cannot change the path. Datasets have no
    /// hourly routes, so an hourly ID is rejected before any request.
    fn export_url(&self, id: &str, tail: &[&str], format: Option<&ExportFormat>) -> Result<Url> {
        let export = export_segments(id)?;
        if export.len() > 1 && !self.hourly {
            return Err(Error::InvalidInput(format!(
                "invalid export ID {id:?}: datasets have only daily exports"
            )));
        }
        let segments = self.path.iter().copied().chain(["export"]);
        let mut url = self.client.endpoint(
            segments
                .chain(export.iter().map(String::as_str))
                .chain(tail.iter().copied()),
        );
        append_query(
            &mut url,
            &[
                ("variant", self.variant.as_ref().map(ExportVariant::as_str)),
                ("format", format.map(ExportFormat::as_str)),
            ],
        );
        Ok(url)
    }
}

/// Appends each pair whose value is set. Calling `query_pairs_mut` with
/// nothing to add would leave an empty `?` on the URL.
fn append_query(url: &mut Url, pairs: &[(&str, Option<&str>)]) {
    for (name, value) in pairs {
        if let Some(value) = value {
            url.query_pairs_mut().append_pair(name, value);
        }
    }
}

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

/// Parses `text` if it is exactly `len` ASCII digits. The digit check rejects
/// a leading `+`, which `str::parse` accepts.
fn digits(text: &str, len: usize) -> Option<u32> {
    if text.len() != len || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

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

/// A redirect from https to http is refused, because following it would send
/// the presigned URL in plain text.
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

/// One page of a feed's or dataset's exports, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportPage {
    /// The stream identifier, such as `proxies`.
    pub stream: String,
    /// The exports on this page. The API names the field `feeds`.
    #[serde(rename = "feeds")]
    pub exports: Vec<Export>,
    /// The cursor of the next page, for [`Feed::list_exports`] or
    /// [`Feed::all_exports`]. It is `None` on the last page.
    #[serde(default, deserialize_with = "non_empty")]
    pub next_cursor: Option<String>,
}

/// Reads an empty cursor as `None`, in case the API ends a listing with one
/// instead of omitting the cursor.
fn non_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.filter(|value| !value.is_empty()))
}

/// An export of a feed or dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Export {
    /// Identifies the export for [`Feed::export_meta`], [`Feed::download`],
    /// and [`Feed::download_url`]. On the first page of a feed, the newest
    /// hourly export may have the ID `latest`. A lite export has the same ID
    /// as the full one.
    pub id: String,
    /// Whether the export covers an hour or a day.
    pub kind: ExportKind,
    /// The UTC day of the export as `YYYY-MM-DD`.
    pub date: String,
    /// The UTC hour of an hourly export, or `None` for a daily export.
    pub hour: Option<u8>,
    /// The size in bytes of the first file in `formats`.
    pub size_bytes: i64,
    /// The number of rows. For WHOIS, it is the sum over `entities`.
    pub row_count: i64,
    /// The hex-encoded SHA-256 of the first file in `formats`.
    pub checksum: String,
    /// When the export was produced.
    pub created_at: Timestamp,
    /// The API path of the export's download, including the version prefix.
    pub download_path: String,
    /// Every file of the export, its own format first. Feeds have one
    /// parquet file.
    pub formats: Vec<ExportFile>,
    /// The parquet files inside a WHOIS archive, or `None` for any other
    /// export.
    pub entities: Option<Vec<ExportEntity>>,
    /// [`ExportVariant::Lite`] on a lite listing, otherwise `None`.
    pub variant: Option<ExportVariant>,
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

/// An export's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportMeta {
    /// Identifies the export.
    pub id: String,
    /// The stream identifier.
    pub stream: String,
    /// Whether the export covers an hour or a day.
    pub kind: ExportKind,
    /// The format that `checksum` and `size` describe, as an
    /// [`ExportFormat`] value: the requested format, or else the export's
    /// own, `parquet` or `tar.gz` for WHOIS.
    pub format: String,
    /// The start of the export's UTC day or hour.
    pub date: Timestamp,
    /// The UTC hour of an hourly export, or `None` for a daily export.
    pub hour: Option<u8>,
    /// The number of rows.
    pub rows: i64,
    /// The size of the file in bytes.
    pub size: i64,
    /// The hex-encoded SHA-256 of the file.
    pub checksum: String,
    /// When the export was produced.
    pub created_at: Timestamp,
    /// The parquet columns. A WHOIS archive has none, and lists them per
    /// file under `entities`.
    pub schema: ParquetSchema,
    /// Every file of the export, its own format first.
    pub formats: Vec<ExportFile>,
    /// The parquet files inside a WHOIS archive, or `None` for any other
    /// export.
    pub entities: Option<Vec<ExportEntity>>,
    /// [`ExportVariant::Lite`] for a lite export, otherwise `None`.
    pub variant: Option<ExportVariant>,
}

/// The columns of a parquet file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ParquetSchema {
    /// The columns in file order.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub fields: Vec<ParquetColumn>,
}

/// The WHOIS meta once sent `"fields": null` for its archive.
fn null_as_empty<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ParquetColumn>, D::Error> {
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

/// A column of a parquet file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ParquetColumn {
    /// The column name.
    pub name: String,
    /// The column type.
    pub r#type: ParquetType,
}

wire_enum! {
    /// The type of a parquet column. Datasets also use types the API does
    /// not list, such as `double` and `list<string>`, which decode as
    /// [`Unknown`](Self::Unknown).
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

/// One file of an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportFile {
    /// The file format.
    pub format: ExportFormat,
    /// The file size in bytes.
    pub size_bytes: i64,
    /// The hex-encoded SHA-256 of the file as downloaded, which for CSV is
    /// the `.csv.gz`.
    pub checksum: String,
}

wire_enum! {
    /// The format of an export file.
    pub enum ExportFormat {
        /// Apache Parquet.
        Parquet = "parquet",
        /// Gzipped CSV with a header row.
        Csv = "csv",
        /// A MaxMind DB.
        Mmdb = "mmdb",
        /// A gzipped tar archive.
        TarGz = "tar.gz",
    }
}

/// A parquet file inside a WHOIS archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportEntity {
    /// The registry objects the file holds.
    pub entity: WhoisEntity,
    /// The number of rows.
    pub row_count: i64,
    /// The file size in bytes.
    pub size_bytes: i64,
    /// The hex-encoded SHA-256 of the file.
    pub checksum: String,
    /// The file's columns. Only [`ExportMeta`] has them.
    pub schema: Option<ParquetSchema>,
}

wire_enum! {
    /// The registry objects in a WHOIS parquet file.
    pub enum WhoisEntity {
        /// Autonomous systems.
        Asns = "asns",
        /// Networks.
        Networks = "networks",
        /// Organisations.
        Organisations = "organisations",
        /// Contacts.
        Contacts = "contacts",
        /// Maintainers.
        Maintainers = "maintainers",
        /// Transfers.
        Transfers = "transfers",
        /// Delegations.
        Delegations = "delegations",
    }
}

wire_enum! {
    /// Which version of an export to read. See [`Feed::with_variant`].
    pub enum ExportVariant {
        /// Every sighting. This is the default.
        Full = "full",
        /// The latest row per key within the export's hour or day, with the
        /// same columns: per address and provider for proxies, per range and
        /// provider for anonymizers, and per address, fingerprint, and
        /// provider for JA4T.
        Lite = "lite",
    }
}

/// The rows and bytes exported per UTC day, from [`Feed::export_volume`]
/// or [`Dataset::export_volume`](crate::Dataset::export_volume).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportVolume {
    /// The stream identifier, such as `proxies`.
    pub stream: String,
    /// The days with exports, oldest first. Today is included while it is
    /// still in progress.
    pub days: Vec<ExportVolumeDay>,
}

/// The exports of one UTC day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportVolumeDay {
    /// The UTC day as `YYYY-MM-DD`.
    pub date: String,
    /// [`ExportKind::Daily`] when the day's daily export is the total, or
    /// [`ExportKind::Hourly`] when its hourly exports were summed, as for
    /// today or a day whose rollup is pending.
    pub kind: ExportKind,
    /// The number of files counted: 1 for a daily export, otherwise the
    /// hourly exports that have landed, out of 24.
    pub snapshots: u32,
    /// The number of rows exported.
    pub row_count: i64,
    /// The number of bytes exported.
    pub size_bytes: i64,
}

struct Pages {
    exports: Exports,
    limit: Option<u32>,
    day: Option<String>,
    cursor: Option<String>,
    done: bool,
    seen: HashSet<String>,
    items: std::vec::IntoIter<Export>,
}

impl Pages {
    async fn next(&mut self) -> Result<Option<Export>> {
        loop {
            if let Some(export) = self.items.next() {
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
                .exports
                .list(self.limit, self.cursor.as_deref(), self.day.as_deref())
                .await?;
            self.items = page.exports.into_iter();
            self.done = page.next_cursor.is_none();
            self.cursor = page.next_cursor;
        }
    }
}

/// A stream of every export of a feed or dataset, newest first. Create one
/// with [`Feed::all_exports`].
pub type ExportStream = EventStream<Export>;

/// The contents of an export file, exactly as served, read in chunks as they
/// arrive. Create one with [`Feed::download`]. Dropping it closes the
/// connection.
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

    const WHOIS_META: &str = r#"{"id":"latest","stream":"whois","kind":"daily","format":"tar.gz","date":1794873600,"rows":23481094,"size":997234948,"checksum":"add0","created_at":1794960929,"schema":{"fields":[]},"formats":[{"format":"tar.gz","size_bytes":997234948,"checksum":"add0"}],"entities":[{"entity":"asns","row_count":123778,"size_bytes":5442736,"checksum":"062f","schema":{"fields":[{"name":"asn","type":"int64"},{"name":"updated","type":"timestamp"}]}},{"entity":"contacts","row_count":6459660,"size_bytes":205447373,"checksum":"cf61","schema":{"fields":[{"name":"emails","type":"list<string>"}]}}]}"#;

    #[test]
    fn decodes_every_file_of_an_export() {
        let item = r#"{"id":"2026-11-17","kind":"daily","date":"2026-11-17","size_bytes":466431938,"row_count":27456467,"checksum":"9712","created_at":1794960045,"download_path":"/api/v4/feeds/geolocation/export/2026-11-17","formats":[{"format":"parquet","size_bytes":466431938,"checksum":"9712"},{"format":"csv","size_bytes":540196440,"checksum":"de74"},{"format":"mmdb","size_bytes":1874587084,"checksum":"ae71"}]}"#;

        let export: Export = serde_json::from_str(item).unwrap();

        let formats: Vec<&ExportFormat> = export.formats.iter().map(|file| &file.format).collect();
        assert_eq!(
            formats,
            [
                &ExportFormat::Parquet,
                &ExportFormat::Csv,
                &ExportFormat::Mmdb
            ]
        );
        assert_eq!(export.formats[2].size_bytes, 1_874_587_084);
        assert_eq!(export.entities, None);
        assert_eq!(export.variant, None);

        let missing = item.replace(r#""formats""#, r#""other""#);
        assert!(serde_json::from_str::<Export>(&missing).is_err());
    }

    #[test]
    fn decodes_a_lite_export() {
        let item = r#"{"id":"2026-11-17","kind":"daily","date":"2026-11-17","size_bytes":10,"row_count":2,"checksum":"ab","created_at":1,"download_path":"/api/v4/feeds/proxies/export/2026-11-17?variant=lite","variant":"lite","formats":[{"format":"parquet","size_bytes":10,"checksum":"ab"}]}"#;

        let export: Export = serde_json::from_str(item).unwrap();

        assert_eq!(export.variant, Some(ExportVariant::Lite));
    }

    #[test]
    fn decodes_a_whois_meta() {
        let meta: ExportMeta = serde_json::from_str(WHOIS_META).unwrap();

        assert_eq!(meta.format, ExportFormat::TarGz.as_str());
        assert!(meta.schema.fields.is_empty());
        assert_eq!(meta.formats[0].format, ExportFormat::TarGz);
        let entities = meta.entities.unwrap();
        assert_eq!(entities[0].entity, WhoisEntity::Asns);
        assert_eq!(entities[1].entity, WhoisEntity::Contacts);
        let columns = &entities[1].schema.as_ref().unwrap().fields;
        assert_eq!(
            columns[0].r#type,
            ParquetType::Unknown("list<string>".into())
        );
    }

    #[test]
    fn a_null_or_missing_schema_has_no_columns() {
        for schema in [r#"{"fields":null}"#, "{}"] {
            let meta = WHOIS_META.replace(r#"{"fields":[]}"#, schema);
            let meta: ExportMeta = serde_json::from_str(&meta).unwrap();
            assert!(meta.schema.fields.is_empty(), "{schema}");
        }
    }

    #[test]
    fn meta_format_takes_every_dataset_format() {
        for format in ["parquet", "csv", "mmdb", "tar.gz", "zip"] {
            let meta = WHOIS_META.replace(
                r#""format":"tar.gz","date""#,
                &format!(r#""format":"{format}","date""#),
            );
            let meta: ExportMeta = serde_json::from_str(&meta).unwrap();
            assert_eq!(meta.format, format);
        }
    }

    #[test]
    fn unknown_formats_and_entities_are_kept() {
        let file: ExportFile =
            serde_json::from_str(r#"{"format":"zip","size_bytes":1,"checksum":"ab"}"#).unwrap();
        assert_eq!(file.format, ExportFormat::Unknown("zip".into()));

        let entity: ExportEntity = serde_json::from_str(
            r#"{"entity":"routes","row_count":1,"size_bytes":1,"checksum":"ab"}"#,
        )
        .unwrap();
        assert_eq!(entity.entity, WhoisEntity::Unknown("routes".into()));
        assert_eq!(entity.schema, None);
    }

    #[test]
    fn dataset_column_types_decode() {
        for name in [
            "double",
            "int32",
            "date",
            "float",
            "timestamp",
            "list<string>",
        ] {
            let column: ParquetColumn =
                serde_json::from_str(&format!(r#"{{"name":"x","type":"{name}"}}"#)).unwrap();
            assert_eq!(column.r#type, ParquetType::Unknown(name.into()));
            assert_eq!(column.r#type.as_str(), name);
        }
    }

    #[test]
    fn decodes_export_volume() {
        let body = r#"{"stream":"proxies","days":[{"date":"2026-11-17","kind":"daily","snapshots":1,"row_count":1528344568,"size_bytes":20640294576},{"date":"2026-11-18","kind":"hourly","snapshots":3,"row_count":6441725000,"size_bytes":880000000}]}"#;

        let volume: ExportVolume = serde_json::from_str(body).unwrap();

        assert_eq!(volume.stream, "proxies");
        assert_eq!(volume.days[0].kind, ExportKind::Daily);
        assert_eq!(volume.days[0].size_bytes, 20_640_294_576);
        assert_eq!(volume.days[1].kind, ExportKind::Hourly);
        assert_eq!(volume.days[1].snapshots, 3);
        assert_eq!(volume.days[1].row_count, 6_441_725_000);
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
