use std::fmt;

use reqwest::Url;

use crate::client::Client;
use crate::error::Result;
use crate::feed::{
    Download, ExportFormat, ExportMeta, ExportPage, ExportStream, ExportVolume, Exports,
};

/// A Synthient dataset, published once a day as files to download. It has no
/// real-time stream and no hourly exports. Get one from
/// [`Client::geolocation`] or [`Client::whois`].
///
/// Exports are identified by `latest`, the newest day, or a UTC day as
/// `YYYY-MM-DD`. An hourly ID returns [`Error::InvalidInput`](crate::Error::InvalidInput)
/// without a request.
///
/// A day has one file per format. Geolocation has [`ExportFormat::Parquet`],
/// the default, [`ExportFormat::Csv`], and [`ExportFormat::Mmdb`]. WHOIS has
/// one [`ExportFormat::TarGz`] archive of parquet files. Pass `None` as
/// `format` for the default file. A day without the requested format returns
/// a 404 [`Error::Api`](crate::Error::Api).
#[derive(Clone)]
pub struct Dataset {
    pub(crate) exports: Exports,
}

impl fmt::Debug for Dataset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dataset")
            .field("path", &self.exports.path.join("/"))
            .finish_non_exhaustive()
    }
}

impl Dataset {
    pub(crate) fn new(client: Client, path: &'static [&'static str]) -> Self {
        Self {
            exports: Exports::new(client, path, false),
        }
    }

    /// Returns one page of the dataset's daily exports, newest first. The
    /// arguments are the same as for [`Feed::list_exports`](crate::Feed::list_exports).
    pub async fn list_exports(
        &self,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<ExportPage> {
        self.exports.list(limit, cursor, None).await
    }

    /// Like [`list_exports`](Self::list_exports), but lists only the export
    /// of `day`, a UTC day as `YYYY-MM-DD`.
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

    /// Like [`all_exports`](Self::all_exports), but streams only the export
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

    /// Returns an export's checksum, size, row count, and the list of its
    /// files. `checksum` and `size` describe the `format` file.
    pub async fn export_meta(&self, id: &str, format: Option<ExportFormat>) -> Result<ExportMeta> {
        self.exports.meta(id, format.as_ref()).await
    }

    /// Returns a presigned URL for an export's `format` file. The URL is
    /// valid for 24 hours and grants access without an API key, so treat it
    /// as a secret.
    pub async fn download_url(&self, id: &str, format: Option<ExportFormat>) -> Result<Url> {
        self.exports.download_url(id, format.as_ref()).await
    }

    /// Opens an export's `format` file for reading, compressed as it is
    /// served. The API key is never sent to the storage host that serves the
    /// file, and errors never include the presigned URL.
    ///
    /// ```no_run
    /// # async fn run(client: synthient::Client) -> Result<(), synthient::Error> {
    /// use synthient::ExportFormat;
    ///
    /// let mut download = client
    ///     .geolocation()
    ///     .download("latest", Some(ExportFormat::Mmdb))
    ///     .await?;
    /// while let Some(chunk) = download.next().await {
    ///     let chunk = chunk?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// A non-success response from the API or the storage host returns an
    /// [`Error::Api`](crate::Error::Api).
    pub async fn download(&self, id: &str, format: Option<ExportFormat>) -> Result<Download> {
        self.exports.download(id, format.as_ref()).await
    }
}
