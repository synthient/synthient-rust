//! Checks the models against the spec the SDK was last updated to, in
//! `.sdkgen/openapi.json`. When the spec changes, update the models and the
//! tables here together.

use std::collections::BTreeSet;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::account::{Account, AccountApiKey, LookupQuota, Organization, Relation, Scope};
use crate::client::Client;
use crate::error::Problem;
use crate::events::*;
use crate::feed::{
    Export, ExportKind, ExportMeta, ExportPage, ParquetColumn, ParquetSchema, ParquetType,
};
use crate::lookup::*;

type Check = fn(Value) -> Result<Value, serde_json::Error>;

/// Maps each spec schema to the Rust type that models it.
const SCHEMAS: &[(&str, Check)] = &[
    ("ASNEntry", round_trip::<AsnCount>),
    ("AccountAPIKey", round_trip::<AccountApiKey>),
    ("AccountInfoResponse", round_trip::<Account>),
    ("ActivityStatsBlock", round_trip::<ActivityStats>),
    ("AnonymizerStreamEntry", round_trip::<AnonymizerEvent>),
    ("CountryEntry", round_trip::<CountryCount>),
    ("DeviceInfo", round_trip::<Device>),
    ("DomainLookupResponse", round_trip::<DomainInfo>),
    ("ExportListItem", round_trip::<Export>),
    ("ExportListResponse", round_trip::<ExportPage>),
    ("ExportMetaResponse", round_trip::<ExportMeta>),
    ("HoneypotADBStreamEntry", round_trip::<HeliosAdbEvent>),
    ("HoneypotDNSStreamEntry", round_trip::<HeliosDnsEvent>),
    ("HoneypotHTTPSStreamEntry", round_trip::<HeliosHttpsEvent>),
    ("HoneypotHTTPStreamEntry", round_trip::<HeliosHttpEvent>),
    ("HoneypotMeta", round_trip::<HeliosMeta>),
    ("IntelligenceInfo", round_trip::<Intelligence>),
    ("IpBatchRequest", round_trip::<BatchRequest>),
    ("IpBatchResponse", round_trip::<BatchResponse>),
    ("IpProvidersResponse", round_trip::<IpInfo>),
    ("JA4TStreamEntry", round_trip::<Ja4tEvent>),
    ("LocationInfo", round_trip::<Location>),
    ("LookupQuota", round_trip::<LookupQuota>),
    ("NetworkInfo", round_trip::<Network>),
    ("Organization", round_trip::<Organization>),
    ("PortEntry", round_trip::<PortCount>),
    ("ProblemDetails", decode::<Problem>),
    ("ProjectedColumn", round_trip::<ParquetColumn>),
    ("ProviderEntry", round_trip::<Provider>),
    ("ProxyStreamEntry", round_trip::<ProxyEvent>),
    ("RecentEvent", round_trip::<DomainEvent>),
    ("Schema", round_trip::<ParquetSchema>),
    ("Stats", round_trip::<DomainStats>),
    ("SubdomainEntry", round_trip::<SubdomainCount>),
    ("TLSClientHello", round_trip::<TlsClientHello>),
    ("TLSCodeName", round_trip::<TlsCodeName>),
    ("TLSExtension", round_trip::<TlsExtension>),
    ("TimeSeriesEntry", round_trip::<TimeSeriesPoint>),
    ("TorrentFileEntry", round_trip::<TorrentFile>),
    ("TorrentPeerEntry", round_trip::<TorrentPeer>),
    ("TorrentStreamEntry", round_trip::<TorrentEvent>),
    ("UniqueIPsBlock", round_trip::<UniqueIps>),
    ("ValidationProblemDetails", decode::<Problem>),
];

/// Error bodies are decoded leniently, so the fields a problem does carry
/// are kept when others are missing.
const LENIENT: &[&str] = &["ProblemDetails", "ValidationProblemDetails"];

/// Maps each spec property with a value set to the values of the Rust enum
/// that models it. A single-value enum, such as a stream's protocol, stays a
/// plain string.
const ENUMS: &[(&str, &str, &[&str])] = &[
    ("AccountInfoResponse", "scopes", Scope::VALUES),
    ("ActivityStatsBlock", "cadence", Cadence::VALUES),
    ("AnonymizerStreamEntry", "type", ProxyType::VALUES),
    ("DeviceInfo", "os", Os::VALUES),
    ("DomainLookupResponse", "status", DomainStatus::VALUES),
    ("ExportListItem", "kind", ExportKind::VALUES),
    ("ExportMetaResponse", "kind", ExportKind::VALUES),
    ("JA4TStreamEntry", "type", ProxyType::VALUES),
    ("NetworkInfo", "type", NetworkType::VALUES),
    ("Organization", "relation", Relation::VALUES),
    ("ProjectedColumn", "type", ParquetType::VALUES),
    ("ProxyStreamEntry", "type", ProxyType::VALUES),
];

fn round_trip<T: DeserializeOwned + Serialize>(value: Value) -> Result<Value, serde_json::Error> {
    serde_json::to_value(serde_json::from_value::<T>(value)?)
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<Value, serde_json::Error> {
    serde_json::from_value::<T>(value.clone())?;
    Ok(value)
}

/// Returns a sample value for a property whose Rust type is deliberately
/// narrower or wider than the spec's, or that must parse as an address.
fn sample_override(location: &str) -> Option<Value> {
    match location {
        // The live Helios streams send tunnel IDs above the int64 maximum,
        // although the spec says int64.
        "HoneypotDNSStreamEntry.tunnel_id"
        | "HoneypotHTTPSStreamEntry.tunnel_id"
        | "HoneypotHTTPStreamEntry.tunnel_id" => Some(json!(u64::MAX)),
        // ASNs are 32-bit, but these two are typed int64 or unbounded.
        "ASNEntry.asn" | "NetworkInfo.asn" => Some(json!(u32::MAX)),
        "IpProvidersResponse.ip" => Some(json!("2001:db8::1")),
        "IpBatchRequest.ips" => Some(json!(["8.8.8.8", "2001:db8::1"])),
        _ => None,
    }
}

fn spec() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.sdkgen/openapi.json");
    let text = std::fs::read_to_string(path).expect("read .sdkgen/openapi.json");
    serde_json::from_str(&text).expect("decode .sdkgen/openapi.json")
}

fn schemas(spec: &Value) -> &Map<String, Value> {
    spec["components"]["schemas"]
        .as_object()
        .expect("spec has schemas")
}

fn properties(schema: &Value) -> impl Iterator<Item = (&String, &Value)> {
    schema["properties"].as_object().into_iter().flatten()
}

fn required(schema: &Value) -> Vec<&str> {
    schema["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn enum_values(schema: &Value) -> Vec<&str> {
    if schema["type"] == "array" {
        return enum_values(&schema["items"]);
    }
    ["enum", "x-extensible-enum"]
        .iter()
        .filter_map(|key| schema[key].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Builds a value for `schema` with every property present. Integers use the
/// extreme of their format, so a Rust type too narrow for the spec fails to
/// decode.
fn sample(spec: &Value, location: &str, schema: &Value) -> Value {
    if let Some(value) = sample_override(location) {
        return value;
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference.trim_start_matches("#/components/schemas/");
        return sample(spec, name, &schemas(spec)[name]);
    }
    if let Some([inner]) = schema["allOf"].as_array().map(Vec::as_slice) {
        return sample(spec, location, inner);
    }
    if let Some(first) = enum_values(schema).first() {
        if schema["type"] != "array" {
            return json!(first);
        }
    }
    match schema["type"].as_str() {
        Some("string") => match schema["format"].as_str() {
            Some("byte") => json!("aGVsbG8="),
            Some("date-time") => json!("2026-09-28T19:04:52.463869Z"),
            Some("uuid") => json!("00000000-0000-0000-0000-000000000000"),
            _ => json!("x"),
        },
        Some("integer") => match (
            &schema["maximum"],
            schema["format"].as_str(),
            schema["minimum"].as_i64(),
        ) {
            (Value::Number(maximum), _, _) => Value::Number(maximum.clone()),
            (_, Some("int64"), Some(0)) => json!(i64::MAX),
            (_, Some("int64"), _) => json!(i64::MIN),
            _ => json!(7),
        },
        Some("number") => json!(1.5),
        Some("boolean") => json!(true),
        Some("array") => json!([sample(spec, location, &schema["items"])]),
        Some("object") if schema["additionalProperties"].is_object() => {
            json!({ "key": sample(spec, location, &schema["additionalProperties"]) })
        }
        Some("object") | None if schema["properties"].is_object() => Value::Object(
            properties(schema)
                .map(|(name, property)| {
                    (
                        name.clone(),
                        sample(spec, &format!("{location}.{name}"), property),
                    )
                })
                .collect(),
        ),
        _ => panic!("{location}: unsupported schema {schema}"),
    }
}

#[test]
fn spec_schemas_are_modeled() {
    let spec = spec();
    let mut failures = Vec::new();

    for (name, schema) in schemas(&spec) {
        let Some((_, check)) = SCHEMAS.iter().find(|(schema, _)| schema == name) else {
            failures.push(format!("schema {name} has no Rust type in SCHEMAS"));
            continue;
        };
        let full = sample(&spec, name, schema);

        match check(full.clone()) {
            Ok(decoded) if decoded == full => {}
            Ok(decoded) => failures.push(format!(
                "{name} round trip changed\n  sent {full}\n   got {decoded}"
            )),
            Err(err) => failures.push(format!("{name} rejected a full sample: {err}\n  {full}")),
        }

        let required = required(schema);
        for (property, value) in properties(schema) {
            let mut without = full.clone();
            without.as_object_mut().unwrap().remove(property);
            let is_required = required.contains(&property.as_str());
            match check(without) {
                Ok(_) if is_required && !LENIENT.contains(&name.as_str()) => {
                    failures.push(format!(
                        "{name}.{property} is required but decodes when missing"
                    ));
                }
                Err(err) if !is_required => {
                    failures.push(format!(
                        "{name}.{property} is optional but fails when missing: {err}"
                    ));
                }
                _ => {}
            }

            if value["nullable"] == true {
                let mut null = full.clone();
                null[property.as_str()] = Value::Null;
                if let Err(err) = check(null) {
                    failures.push(format!(
                        "{name}.{property} is nullable but rejects null: {err}"
                    ));
                }
            }
        }
    }
    for (name, _) in SCHEMAS {
        if !schemas(&spec).contains_key(*name) {
            failures.push(format!(
                "SCHEMAS maps {name}, which is no longer in the spec"
            ));
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn spec_enums_are_modeled() {
    let spec = spec();
    let mut failures = Vec::new();

    for (name, schema) in schemas(&spec) {
        for (property, value) in properties(schema) {
            let mut want = enum_values(value);
            if want.len() < 2 {
                continue;
            }
            let Some((_, _, values)) = ENUMS.iter().find(|(s, p, _)| s == name && *p == property)
            else {
                failures.push(format!(
                    "{name}.{property} has a value set but no enum in ENUMS"
                ));
                continue;
            };
            let mut got = values.to_vec();
            want.sort_unstable();
            got.sort_unstable();
            if got != want {
                failures.push(format!(
                    "{name}.{property} values are {want:?}, but the enum has {got:?}"
                ));
            }
        }
    }
    for (name, property, _) in ENUMS {
        if enum_values(&schemas(&spec)[*name]["properties"][*property]).is_empty() {
            failures.push(format!(
                "ENUMS maps {name}.{property}, which has no value set in the spec"
            ));
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn spec_operations_are_implemented() {
    let spec = spec();
    let client = Client::new("key").unwrap();

    let mut implemented: BTreeSet<String> = [
        "GET /account/me",
        "GET /health",
        "GET /lookup/domain/{domain}",
        "GET /lookup/ip/{ip_address}",
        "POST /lookup/ips",
    ]
    .map(String::from)
    .into();
    let feeds = [
        client.proxies().path,
        client.anonymizers().path,
        client.torrents().path,
        client.ja4t().path,
        client.helios_http().path,
        client.helios_https().path,
        client.helios_dns().path,
        client.helios_adb().path,
    ];
    for path in feeds {
        for suffix in [
            "/export",
            "/export/{date}",
            "/export/{date}/meta",
            "/export/{date}/{hour}",
            "/export/{date}/{hour}/meta",
            "/stream",
        ] {
            implemented.insert(format!("GET /{}{suffix}", path.join("/")));
        }
    }

    let operations: BTreeSet<String> = spec["paths"]
        .as_object()
        .unwrap()
        .iter()
        .flat_map(|(path, methods)| {
            methods
                .as_object()
                .unwrap()
                .keys()
                .map(move |method| format!("{} {path}", method.to_uppercase()))
        })
        .collect();

    let missing: Vec<_> = operations.difference(&implemented).collect();
    let extra: Vec<_> = implemented.difference(&operations).collect();
    assert!(
        missing.is_empty(),
        "spec operations not implemented: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "implemented operations not in the spec: {extra:?}"
    );
}
