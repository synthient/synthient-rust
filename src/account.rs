use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::client::{Client, Request};
use crate::error::Result;
use crate::retry::RetryPolicy;

/// The account that owns the API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Account {
    /// The account holder's email address.
    pub email: String,
    /// The account holder's first name.
    pub first_name: String,
    /// The account holder's last name.
    pub last_name: String,
    /// The organization the account belongs to.
    pub organization: Organization,
    /// The permissions granted to the API key.
    pub scopes: Vec<Scope>,
    /// The remaining lookup credit balance.
    pub lookup_quota: LookupQuota,
    /// The API key used for the request, or `None` if it was deleted while
    /// the request was in flight.
    pub api_key: Option<AccountApiKey>,
}

/// The organization an account belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Organization {
    /// The organization's UUID.
    pub id: String,
    /// The organization's name.
    pub name: String,
    /// The account's role in the organization.
    pub relation: Relation,
}

wire_enum! {
    /// An account's role in its organization.
    pub enum Relation {
        /// The organization's owner.
        Owner = "OWNER",
        /// An administrator.
        Admin = "ADMIN",
        /// A member.
        Member = "MEMBER",
    }
}

wire_enum! {
    /// A permission granted to an API key.
    ///
    /// [`Basic`](Self::Basic) authorizes lookups and the account. Each feed
    /// has one scope for its exports and one for its stream, such as
    /// [`TorrentsFeed`](Self::TorrentsFeed) and
    /// [`TorrentsStream`](Self::TorrentsStream). The proxy feed's are
    /// [`ProxyFeeds`](Self::ProxyFeeds) and
    /// [`ProxyFirehose`](Self::ProxyFirehose). Datasets have only an export
    /// scope, such as [`GeolocationFeed`](Self::GeolocationFeed).
    pub enum Scope {
        /// Lookups and account details.
        Basic = "BASIC",
        /// Proxy feed exports.
        ProxyFeeds = "PROXY_FEEDS",
        /// The proxy feed stream.
        ProxyFirehose = "PROXY_FIREHOSE",
        /// Anonymizer feed exports.
        AnonymizersFeed = "ANONYMIZERS_FEED",
        /// The anonymizer feed stream.
        AnonymizersStream = "ANONYMIZERS_STREAM",
        /// JA3 feed exports.
        Ja3Feed = "JA3_FEED",
        /// The JA3 feed stream.
        Ja3Stream = "JA3_STREAM",
        /// Torrent feed exports.
        TorrentsFeed = "TORRENTS_FEED",
        /// The torrent feed stream.
        TorrentsStream = "TORRENTS_STREAM",
        /// Helios HTTP feed exports.
        HoneypotHttpFeed = "HONEYPOT_HTTP_FEED",
        /// The Helios HTTP feed stream.
        HoneypotHttpStream = "HONEYPOT_HTTP_STREAM",
        /// Helios HTTPS feed exports.
        HoneypotHttpsFeed = "HONEYPOT_HTTPS_FEED",
        /// The Helios HTTPS feed stream.
        HoneypotHttpsStream = "HONEYPOT_HTTPS_STREAM",
        /// Helios ADB feed exports.
        HoneypotAdbFeed = "HONEYPOT_ADB_FEED",
        /// The Helios ADB feed stream.
        HoneypotAdbStream = "HONEYPOT_ADB_STREAM",
        /// Helios DNS feed exports.
        HoneypotDnsFeed = "HONEYPOT_DNS_FEED",
        /// The Helios DNS feed stream.
        HoneypotDnsStream = "HONEYPOT_DNS_STREAM",
        /// JA4 feed exports.
        Ja4Feed = "JA4_FEED",
        /// The JA4 feed stream.
        Ja4Stream = "JA4_STREAM",
        /// JA4T feed exports.
        Ja4tFeed = "JA4T_FEED",
        /// The JA4T feed stream.
        Ja4tStream = "JA4T_STREAM",
        /// Helios dashboard access. It authorizes no API operation.
        Helios = "HELIOS",
        /// WHOIS dataset exports.
        WhoisFeed = "WHOIS_FEED",
        /// Reserved for a WHOIS stream. No operation accepts it yet.
        WhoisStream = "WHOIS_STREAM",
        /// Geolocation dataset exports.
        GeolocationFeed = "GEOLOCATION_FEED",
        /// Reserved for the exports of an upcoming network IOC feed.
        NetworkIocsFeed = "NETWORK_IOCS_FEED",
        /// Reserved for the stream of an upcoming network IOC feed.
        NetworkIocsStream = "NETWORK_IOCS_STREAM",
        /// Reserved for the exports of an upcoming file IOC feed.
        FileIocsFeed = "FILE_IOCS_FEED",
        /// Reserved for the stream of an upcoming file IOC feed.
        FileIocsStream = "FILE_IOCS_STREAM",
        /// Internal proxy administration.
        #[deprecated(
            note = "ADMIN_PROXIES was removed from the Synthient API and never authorized a public operation. It will be removed in the next major version."
        )]
        AdminProxies = "ADMIN_PROXIES",
    }
}

/// An account's lookup credit balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LookupQuota {
    /// The number of lookup credits remaining.
    pub credits: i64,
    /// The number of seconds until the credits reset.
    pub resets_in: i64,
}

/// An API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountApiKey {
    /// The key's UUID. It is not the secret key itself.
    pub id: String,
    /// The key's display name.
    pub name: String,
    /// When the key was created, as an RFC 3339 timestamp.
    pub created_at: String,
}

impl Client {
    /// Returns the account that owns the API key, including its scopes and
    /// remaining lookup credits. It costs no credits.
    pub async fn account(&self) -> Result<Account> {
        let url = self.endpoint(["account", "me"]);
        self.request_json(Request::get(url, RetryPolicy::Transient))
            .await
    }

    /// Checks that the API and its dependencies are healthy. It costs no
    /// credits, and is retried only on 429 responses that carry Retry-After.
    ///
    /// # Errors
    ///
    /// An unhealthy API returns a 503 [`Error::Api`](crate::Error::Api). It
    /// needs a valid API key, with any scope, so a malformed or unknown key
    /// returns a 401.
    pub async fn health(&self) -> Result<()> {
        let url = self.endpoint(["health"]);
        let _: IgnoredAny = self
            .request_json(Request::get(url, RetryPolicy::RateLimit))
            .await?;
        Ok(())
    }
}
