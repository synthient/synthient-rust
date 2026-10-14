use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::timestamp::Timestamp;

wire_enum! {
    /// The kind of service seen on an address.
    pub enum ProxyType {
        /// A free VPN.
        FreeVpn = "FREE_VPN",
        /// A commercial VPN.
        CommercialVpn = "COMMERCIAL_VPN",
        /// An enterprise VPN.
        EnterpriseVpn = "ENTERPRISE_VPN",
        /// A mobile proxy.
        MobileProxy = "MOBILE_PROXY",
        /// A blockchain-based proxy network.
        BlockchainProxy = "BLOCKCHAIN_PROXY",
        /// A residential proxy.
        ResidentialProxy = "RESIDENTIAL_PROXY",
        /// A public proxy.
        PublicProxy = "PUBLIC_PROXY",
        /// A datacenter proxy.
        DatacenterProxy = "DATACENTER_PROXY",
        /// A Tor node.
        TorNode = "TOR_NODE",
        /// A private relay, such as iCloud Private Relay.
        PrivateRelay = "PRIVATE_RELAY",
        /// A botnet.
        Botnet = "BOTNET",
        /// A search engine crawler.
        SearchEngine = "SEARCH_ENGINE",
        /// An AI crawler.
        AiCrawler = "AI_CRAWLER",
        /// A social media crawler.
        SocialMedia = "SOCIAL_MEDIA",
        /// An uptime monitor.
        UptimeMonitor = "UPTIME_MONITOR",
        /// A link preview fetcher.
        LinkPreview = "LINK_PREVIEW",
        /// An SEO crawler.
        SeoCrawler = "SEO_CRAWLER",
        /// A web archiver.
        WebArchiver = "WEB_ARCHIVER",
        /// A webhook provider.
        WebhookProvider = "WEBHOOK_PROVIDER",
        /// A payment processor.
        PaymentProcessor = "PAYMENT_PROCESSOR",
        /// An internet scanner.
        InternetScanner = "INTERNET_SCANNER",
    }
}

/// A proxy sighting from the proxies feed. The API may omit any field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProxyEvent {
    /// The proxy exit address.
    pub ip: Option<String>,
    /// The proxy service tag.
    pub provider: Option<String>,
    /// The proxy category.
    pub r#type: Option<ProxyType>,
    /// The ISO 3166-1 alpha-2 country of the address.
    pub country_code: Option<String>,
    /// The autonomous system number of the address.
    pub asn: Option<u32>,
    /// When the proxy was seen, in Unix seconds.
    pub timestamp: Option<Timestamp>,
}

/// A VPN or anonymizer IP range from the anonymizers feed. The API may omit
/// any field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AnonymizerEvent {
    /// The first address of the range.
    pub range_start: Option<String>,
    /// The last address of the range.
    pub range_end: Option<String>,
    /// The service tag.
    pub provider: Option<String>,
    /// The service category, such as [`ProxyType::CommercialVpn`].
    pub r#type: Option<ProxyType>,
    /// The ISO 3166-1 alpha-2 country of `range_start`.
    pub country_code: Option<String>,
    /// The autonomous system number of `range_start`.
    pub asn: Option<u32>,
    /// When the range was seen, in Unix milliseconds.
    pub timestamp: Option<Timestamp>,
}

/// A torrent observed on the BitTorrent DHT or a tracker. The API may omit
/// any field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TorrentEvent {
    /// The hex-encoded SHA-1 info hash.
    pub info_hash: Option<String>,
    /// The torrent name.
    pub name: Option<String>,
    /// The torrent's magnet link.
    pub magnet_uri: Option<String>,
    /// The total size of the torrent's files in bytes.
    pub total_size: Option<i64>,
    /// The piece size in bytes.
    pub piece_length: Option<i64>,
    /// The number of files in the torrent.
    pub file_count: Option<i64>,
    /// The torrent's files.
    pub files: Option<Vec<TorrentFile>>,
    /// The peers seen sharing the torrent.
    pub peers: Option<Vec<TorrentPeer>>,
    /// When the torrent was seen, in Unix seconds.
    pub timestamp: Option<Timestamp>,
}

/// A file in a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TorrentFile {
    /// The file path within the torrent.
    pub path: Option<String>,
    /// The file size in bytes.
    pub length: Option<i64>,
}

/// A peer seen sharing a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TorrentPeer {
    /// The peer's address.
    pub ip: Option<String>,
    /// The peer's port.
    pub port: Option<u16>,
    /// Where the peer was found, such as `DHT`, `PEX`, or `tracker`.
    pub source: Option<String>,
    /// Whether the peer supports encrypted connections.
    pub encrypted: Option<bool>,
}

/// A JA4T TCP fingerprint sighting. The API may omit any field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Ja4tEvent {
    /// The address the fingerprint was seen from.
    pub ip: Option<String>,
    /// The JA4T fingerprint.
    pub ja4t: Option<String>,
    /// The service tag of the address.
    pub provider: Option<String>,
    /// The service category, such as [`ProxyType::DatacenterProxy`].
    pub r#type: Option<ProxyType>,
    /// The ISO 3166-1 alpha-2 country of the address.
    pub country_code: Option<String>,
    /// The autonomous system number of the address.
    pub asn: Option<u32>,
    /// When the fingerprint was seen, in Unix seconds.
    pub timestamp: Option<Timestamp>,
}

/// The Helios honeypot sensor that captured an event. The API may omit any
/// field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeliosMeta {
    /// The proxy exit address the traffic arrived through.
    pub proxy_ip: Option<String>,
    /// The sensor that captured the traffic.
    pub server: Option<String>,
    /// The sensor pool.
    pub pool_id: Option<String>,
    /// The proxy service the traffic arrived through.
    pub provider: Option<String>,
}

/// An HTTP request captured by a Helios honeypot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeliosHttpEvent {
    /// When the request was captured, in Unix milliseconds.
    pub timestamp: Timestamp,
    /// The proxy tunnel the request arrived through. Live values use the
    /// full `u64` range.
    pub tunnel_id: u64,
    /// The host the tunnel was opened to.
    pub domain: String,
    /// The destination port.
    pub port: u16,
    /// Always `http`.
    pub protocol: String,
    /// The capturing sensor.
    pub meta: HeliosMeta,
    /// The parsed request.
    pub details: Option<HttpRequestDetails>,
    /// The raw request bytes, which the API sends as base64.
    #[serde(default, with = "base64_bytes")]
    pub raw: Option<Vec<u8>>,
}

/// A parsed HTTP request. The API may omit any field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HttpRequestDetails {
    /// The request method.
    pub method: Option<String>,
    /// The request target.
    pub uri: Option<String>,
    /// The HTTP version, such as `HTTP/1.1`.
    pub version: Option<String>,
    /// The request headers.
    pub headers: Option<BTreeMap<String, String>>,
}

/// A TLS ClientHello captured by a Helios honeypot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeliosHttpsEvent {
    /// When the handshake was captured, in Unix milliseconds.
    pub timestamp: Timestamp,
    /// The proxy tunnel the handshake arrived through. Live values use the
    /// full `u64` range.
    pub tunnel_id: u64,
    /// The host the tunnel was opened to.
    pub domain: String,
    /// The destination port.
    pub port: u16,
    /// Always `https`.
    pub protocol: String,
    /// The capturing sensor.
    pub meta: HeliosMeta,
    /// The parsed ClientHello, or `None` if the sensor could not parse the
    /// handshake.
    pub details: Option<TlsClientHello>,
    /// Always `None`, because the sensors do not capture handshake bytes.
    pub raw: Option<String>,
}

/// A TLS ClientHello parsed by a Helios sensor. Fields the sensor could not
/// parse are `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TlsClientHello {
    /// The record layer version, such as `TLS 1.0`.
    pub record_version: Option<String>,
    /// The ClientHello version, such as `TLS 1.2`.
    pub handshake_version: Option<String>,
    /// The hex-encoded 32-byte client random.
    pub client_random: Option<String>,
    /// The hex-encoded session ID.
    pub session_id: Option<String>,
    /// The length of the session ID in bytes.
    pub session_id_length: Option<u8>,
    /// The offered cipher suites.
    pub cipher_suites: Option<Vec<TlsCodeName>>,
    /// The offered compression methods.
    pub compression_methods: Option<Vec<u8>>,
    /// The extensions in wire order.
    pub extensions: Option<Vec<TlsExtension>>,
    /// The `server_name` extension value.
    pub sni: Option<String>,
    /// The offered application protocols.
    pub alpn: Option<Vec<String>>,
    /// The offered TLS versions.
    pub supported_versions: Option<Vec<String>>,
    /// The offered key exchange groups.
    pub supported_groups: Option<Vec<TlsCodeName>>,
    /// The offered signature algorithms.
    pub signature_algorithms: Option<Vec<TlsCodeName>>,
    /// The offered elliptic curve point formats.
    pub ec_point_formats: Option<Vec<String>>,
    /// The groups the client sent key shares for.
    pub key_share_groups: Option<Vec<String>>,
    /// The offered pre-shared key exchange modes.
    pub psk_key_exchange_modes: Option<Vec<String>>,
    /// The offered certificate compression algorithms.
    pub compress_certificate: Option<Vec<String>>,
    /// Whether the ClientHello contains GREASE values.
    pub has_grease: Option<bool>,
    /// Whether the client offered application settings (ALPS).
    pub application_settings: Option<bool>,
    /// Whether the client offered delegated credentials.
    pub delegated_credentials: Option<bool>,
    /// Whether the client offered encrypt-then-MAC.
    pub encrypt_then_mac: Option<bool>,
    /// Whether the client offered the extended master secret.
    pub extended_master_secret: Option<bool>,
    /// Whether the client offered post-handshake authentication.
    pub post_handshake_auth: Option<bool>,
    /// Whether the client sent renegotiation info.
    pub renegotiation_info: Option<bool>,
    /// Whether the client requested signed certificate timestamps.
    pub signed_certificate_timestamps: Option<bool>,
    /// Whether the client requested OCSP stapling.
    pub status_request: Option<bool>,
}

/// A TLS registry code point and its name. GREASE values are named
/// `GREASE(0x....)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TlsCodeName {
    /// The code point.
    pub code: Option<u16>,
    /// The code point's registered name.
    pub name: Option<String>,
}

/// An extension in a TLS ClientHello.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TlsExtension {
    /// The extension type.
    pub code: Option<u16>,
    /// The extension's registered name.
    pub name: Option<String>,
    /// The extension body length in bytes.
    pub length: Option<u16>,
}

/// A destination that a Helios honeypot tunnel was asked to reach. Despite
/// the feed's name, it is not a DNS query: names resolved inside a tunneled
/// DNS session are not visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeliosDnsEvent {
    /// When the event was captured, in Unix milliseconds.
    pub timestamp: Timestamp,
    /// The proxy tunnel. Live values use the full `u64` range.
    pub tunnel_id: u64,
    /// The host name or address literal the tunnel was opened to.
    pub domain: String,
    /// The destination port.
    pub port: u16,
    /// Always `dns`.
    pub protocol: String,
    /// The capturing sensor.
    pub meta: HeliosMeta,
}

/// An Android Debug Bridge shell command captured by a Helios honeypot. It
/// has no timestamp because the sensor sends none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeliosAdbEvent {
    /// The ADB session as 32 hex characters.
    pub session: String,
    /// The 1-based position of the command within its session.
    pub sequential_id: u64,
    /// The shell command. Invalid UTF-8 bytes are replaced with U+FFFD.
    pub command: String,
    /// The hex-encoded SHA-256 the sensor assigns to the session's payload.
    /// It is not a hash of `command`: every command in a session shares it,
    /// and sessions that replay the same script share it too.
    pub hash: String,
}

mod base64_bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        bytes: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(bytes) => serializer.serialize_some(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let Some(text) = Option::<String>::deserialize(deserializer)? else {
            return Ok(None);
        };
        STANDARD
            .decode(text)
            .map(Some)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tunnel_ids_above_i64_max() {
        let line = r#"{"timestamp":1778200137487,"tunnel_id":17257744507466107949,"domain":"example.com","port":443,"protocol":"dns","meta":{}}"#;

        let event: HeliosDnsEvent = serde_json::from_str(line).unwrap();

        assert_eq!(event.tunnel_id, 17_257_744_507_466_107_949);
        assert!(
            serde_json::to_string(&event)
                .unwrap()
                .contains("17257744507466107949")
        );
    }

    #[test]
    fn decodes_base64_raw_requests() {
        let line = r#"{"timestamp":1,"tunnel_id":1,"domain":"d","port":80,"protocol":"http","meta":{},"raw":"R0VUIC8gSFRUUC8xLjENCg=="}"#;
        let event: HeliosHttpEvent = serde_json::from_str(line).unwrap();
        assert_eq!(event.raw.as_deref(), Some(&b"GET / HTTP/1.1\r\n"[..]));

        let missing =
            r#"{"timestamp":1,"tunnel_id":1,"domain":"d","port":80,"protocol":"http","meta":{}}"#;
        let event: HeliosHttpEvent = serde_json::from_str(missing).unwrap();
        assert_eq!(event.raw, None);

        let invalid = r#"{"timestamp":1,"tunnel_id":1,"domain":"d","port":80,"protocol":"http","meta":{},"raw":"not base64!"}"#;
        assert!(serde_json::from_str::<HeliosHttpEvent>(invalid).is_err());
    }

    #[test]
    fn unknown_enum_values_are_kept() {
        let event: ProxyEvent = serde_json::from_str(r#"{"type":"NEW_KIND"}"#).unwrap();

        assert_eq!(event.r#type, Some(ProxyType::Unknown("NEW_KIND".into())));
        assert_eq!(
            serde_json::to_string(&event.r#type).unwrap(),
            r#""NEW_KIND""#
        );
    }

    #[test]
    fn enums_display_with_padding() {
        assert_eq!(format!("[{:<10}]", ProxyType::FreeVpn), "[FREE_VPN  ]");
        assert_eq!(
            format!("[{:>5}]", ProxyType::Unknown("NEW".into())),
            "[  NEW]"
        );
    }

    #[test]
    fn wrong_types_are_rejected() {
        assert!(serde_json::from_str::<ProxyEvent>(r#"{"asn":"13335"}"#).is_err());
        assert!(serde_json::from_str::<ProxyEvent>(r#"{"asn":4294967296}"#).is_err());
        assert!(serde_json::from_str::<ProxyEvent>(r#"{"type":7}"#).is_err());
    }
}
