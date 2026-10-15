use std::net::IpAddr;

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::{Client, Request};
use crate::error::{Error, Result};
use crate::retry::RetryPolicy;
use crate::timestamp::Timestamp;

const MAX_BATCH_SIZE: usize = 1000;

/// The intelligence Synthient holds for an IP address. `network`, `location`,
/// and `intelligence` are `None` when the API has no data for them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct IpInfo {
    /// The address that was looked up.
    pub ip: IpAddr,
    /// The network that announces the address.
    pub network: Option<Network>,
    /// The address's geolocation.
    pub location: Option<Location>,
    /// The risk score and the proxy, VPN, and device observations for the
    /// address.
    pub intelligence: Option<Intelligence>,
}

/// The network that announces an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Network {
    /// The autonomous system number, or 0 if the address is not routed.
    pub asn: u32,
    /// The name of the internet service provider.
    pub isp: String,
    /// The network type.
    pub r#type: NetworkType,
    /// The organization the address is registered to.
    pub org: Option<String>,
    /// The network operator's domain.
    pub domain: Option<String>,
    /// The operator's abuse contact email.
    pub abuse_email: Option<String>,
    /// The operator's abuse contact phone number.
    pub abuse_phone: Option<String>,
}

wire_enum! {
    /// The kind of network that announces an address.
    pub enum NetworkType {
        /// A mobile carrier network.
        Mobile = "MOBILE",
        /// A satellite network.
        Satellite = "SATELLITE",
        /// An in-flight Wi-Fi network.
        InFlightWifi = "IN_FLIGHT_WIFI",
        /// A residential broadband network.
        Residential = "RESIDENTIAL",
        /// A corporate network.
        Corporate = "CORPORATE",
        /// An academic network.
        Academic = "ACADEMIC",
        /// A hosting or cloud network.
        Datacenter = "DATACENTER",
        /// A government network.
        Government = "GOVERNMENT",
    }
}

/// The geolocation of an address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Location {
    /// The ISO 3166-1 alpha-2 country code.
    pub country: String,
    /// The state or region.
    pub state: String,
    /// The city.
    pub city: String,
    /// The IANA time zone name.
    pub timezone: String,
    /// The latitude in degrees.
    pub latitude: f64,
    /// The longitude in degrees.
    pub longitude: f64,
    /// The geohash of the coordinates.
    pub geo_hash: String,
}

/// Synthient's assessment of an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Intelligence {
    /// The risk of the address, from 0 to 100.
    pub risk_score: u8,
    /// The kinds of services seen on the address, such as
    /// `RESIDENTIAL_PROXY`. New values may be added.
    pub categories: Vec<String>,
    /// Behaviors observed from the address. New values may be added.
    pub behavior: Vec<String>,
    /// The proxy, VPN, and anonymizer services seen using the address. An
    /// empty list does not mean the address is clean.
    pub providers: Vec<Provider>,
    /// The device types seen behind the address.
    pub devices: Vec<Device>,
}

/// A proxy, VPN, or anonymizer service seen using an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Provider {
    /// The service tag, such as `BRIGHTDATA`.
    #[serde(rename = "provider")]
    pub name: String,
    /// The service category, such as `RESIDENTIAL_PROXY`.
    pub r#type: String,
    /// When the service was last seen using the address.
    pub last_seen: Timestamp,
}

/// A device type seen behind an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Device {
    /// The operating system.
    pub os: Os,
    /// The operating system version.
    pub version: String,
    /// When the device was last seen.
    pub last_seen: Timestamp,
}

wire_enum! {
    /// A device operating system.
    pub enum Os {
        /// Android.
        Android = "Android",
        /// ChromeOS.
        ChromeOs = "Chrome OS",
        /// FreeBSD.
        FreeBsd = "FreeBSD",
        /// iOS.
        Ios = "iOS",
        /// Linux.
        Linux = "Linux",
        /// macOS.
        MacOsX = "Mac OS X",
        /// Windows.
        Windows = "Windows",
    }
}

/// The honeypot traffic Synthient has observed for a domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainInfo {
    /// The domain that was looked up.
    pub domain: String,
    /// Whether the domain had events in the last 24 hours.
    pub status: DomainStatus,
    /// Aggregate event counts.
    pub stats: DomainStats,
    /// Counts of distinct source addresses.
    pub unique_ips: UniqueIps,
    /// Daily event counts.
    pub time_series: Vec<TimeSeriesPoint>,
    /// The network that sent the most events, or `None` when there are no
    /// events.
    pub top_asn: Option<AsnCount>,
    /// The most targeted subdomains.
    pub top_subdomains: Vec<SubdomainCount>,
    /// The most targeted ports.
    pub top_ports: Vec<PortCount>,
    /// Event counts by source country.
    pub geo_distribution: Vec<CountryCount>,
    /// The last 30 days of events counted by UTC day of week and hour,
    /// indexed as `hour_dow_heatmap[day][hour]`. Day 0 is Monday.
    pub hour_dow_heatmap: Vec<Vec<u64>>,
    /// The hourly activity pattern.
    pub activity_stats: ActivityStats,
    /// The most recent events, capped by the `limit` passed to
    /// [`Client::lookup_domain`].
    pub recent_events: Vec<DomainEvent>,
}

wire_enum! {
    /// Whether a domain has recent activity.
    pub enum DomainStatus {
        /// The domain had events in the last 24 hours.
        Active = "active",
        /// The domain had no events in the last 24 hours, or has never been
        /// observed.
        Inactive = "inactive",
    }
}

/// Aggregate event counts for a domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainStats {
    /// The number of events in the last 24 hours.
    pub events_24h: u64,
    /// The number of events in the last 30 days.
    pub total_events_30d: u64,
}

/// Counts of the distinct source addresses seen for a domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UniqueIps {
    /// The count for the last 24 hours.
    pub value_24h: u64,
    /// The count for the last 30 days.
    pub value_30d: u64,
    /// Hourly counts for the last 24 hours.
    pub sparkline_24h: Vec<u64>,
}

/// The event counts for one day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TimeSeriesPoint {
    /// The start of the day.
    pub date: Timestamp,
    /// The number of events that day.
    pub events: u64,
    /// The number of distinct source addresses that day.
    pub unique_ips: u64,
}

/// An autonomous system and its event count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AsnCount {
    /// The autonomous system number.
    pub asn: u32,
    /// The number of events from the network.
    pub events: u64,
}

/// A subdomain and its event count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubdomainCount {
    /// The targeted subdomain.
    pub subdomain: String,
    /// The number of events that targeted it.
    pub count: u64,
}

/// A port and its event count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PortCount {
    /// The targeted port.
    pub port: u16,
    /// The number of events that targeted it.
    pub count: u64,
}

/// A source country and its counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CountryCount {
    /// The ISO 3166-1 alpha-2 country code.
    pub country_code: String,
    /// The number of events from the country.
    pub events: u64,
    /// The number of distinct addresses from the country.
    pub unique_ips: u64,
}

/// The hourly activity pattern of a domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ActivityStats {
    /// How events are spread over the last 30 days.
    pub cadence: Cadence,
    /// The median number of events per hour.
    pub median_per_hour: u64,
    /// The 95th percentile of events per hour.
    pub p95_per_hour: u64,
    /// The busiest UTC hour of the day.
    pub peak_hour: u8,
    /// The quietest UTC hour of the day.
    pub quiet_hour: u8,
}

wire_enum! {
    /// How a domain's events are spread over time. The API sends each value
    /// as an English sentence.
    pub enum Cadence {
        /// "No activity in the last 30 days."
        NoActivity = "No activity in the last 30 days.",
        /// "Activity is evenly distributed across the day."
        EvenlyDistributed = "Activity is evenly distributed across the day.",
        /// "Activity follows a weekday business-hours pattern."
        BusinessHours = "Activity follows a weekday business-hours pattern.",
        /// "Activity follows a weekend pattern."
        Weekend = "Activity follows a weekend pattern.",
        /// "Activity is burst-driven with significant peaks."
        BurstDriven = "Activity is burst-driven with significant peaks.",
    }
}

/// A single honeypot event for a domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DomainEvent {
    /// When the event happened.
    pub timestamp: Timestamp,
    /// The proxy exit address that carried the event. The API names the
    /// field `source_ip_masked`, but the address is not masked.
    #[serde(rename = "source_ip_masked")]
    pub source_ip: String,
    /// The subdomain that was targeted.
    pub target_subdomain: String,
    /// The source country.
    pub country_code: String,
    /// The targeted port.
    pub port: u16,
}

#[derive(Serialize)]
#[cfg_attr(test, derive(Deserialize))]
pub(crate) struct BatchRequest {
    pub(crate) ips: Vec<IpAddr>,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(Serialize))]
pub(crate) struct BatchResponse {
    pub(crate) results: Vec<IpInfo>,
}

impl Client {
    /// Returns intelligence for an IPv4 or IPv6 address. Each lookup costs
    /// one credit, and failed lookups are not billed.
    ///
    /// # Errors
    ///
    /// Private and reserved addresses return a 422 [`Error::Api`].
    pub async fn lookup_ip(&self, ip: impl Into<IpAddr>) -> Result<IpInfo> {
        let ip = ip.into().to_string();
        let url = self.endpoint(["lookup", "ip", ip.as_str()]);
        self.request_json(Request::get(url, RetryPolicy::RateLimit))
            .await
    }

    /// Looks up as many as 1000 addresses in one request. An empty slice
    /// returns an empty result without a request.
    ///
    /// Private and reserved addresses are skipped and duplicates collapse
    /// into one result, so there can be fewer results than addresses. Match
    /// results by [`IpInfo::ip`] rather than by index. The API bills
    /// `ceil(n * 0.9)` credits for the `n` distinct addresses it looks up,
    /// and bills nothing if the account cannot cover the whole batch.
    ///
    /// # Errors
    ///
    /// More than 1000 addresses return [`Error::InvalidInput`] without a
    /// request. A batch with no address the API can look up returns a 422
    /// [`Error::Api`].
    pub async fn lookup_ips(&self, ips: &[IpAddr]) -> Result<Vec<IpInfo>> {
        if ips.is_empty() {
            return Ok(Vec::new());
        }
        if ips.len() > MAX_BATCH_SIZE {
            return Err(Error::InvalidInput(format!(
                "{} addresses exceed the batch limit of {MAX_BATCH_SIZE}",
                ips.len()
            )));
        }

        let body = serde_json::to_vec(&BatchRequest { ips: ips.to_vec() })?;
        let request = Request {
            method: Method::POST,
            body: Some(body.into()),
            ..Request::get(self.endpoint(["lookup", "ips"]), RetryPolicy::RateLimit)
        };
        let response: BatchResponse = self.request_json(request).await?;
        Ok(response.results)
    }

    /// Returns the honeypot traffic observed for a domain. `limit` caps the
    /// number of recent events returned, from 1 to 1000, and the API default
    /// is 50. Internationalized names must be sent in Punycode. A domain that
    /// has never been observed returns [`DomainStatus::Inactive`] and zero
    /// counts rather than an error. Each lookup costs one credit.
    ///
    /// ```no_run
    /// # async fn run(client: synthient::Client) -> Result<(), synthient::Error> {
    /// let info = client.lookup_domain("example.com", Some(10)).await?;
    /// println!("{} events in 24h", info.stats.events_24h);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn lookup_domain(&self, domain: &str, limit: Option<u32>) -> Result<DomainInfo> {
        if matches!(domain, "" | "." | "..") {
            return Err(Error::InvalidInput(format!("invalid domain {domain:?}")));
        }
        let mut url = self.endpoint(["lookup", "domain", domain]);
        if let Some(limit) = limit {
            url.query_pairs_mut()
                .append_pair("limit", &limit.to_string());
        }
        self.request_json(Request::get(url, RetryPolicy::RateLimit))
            .await
    }
}
