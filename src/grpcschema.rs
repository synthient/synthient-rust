//! Fetches the protobuf schema of the Synthient gRPC API using gRPC server
//! reflection. It needs the `grpcschema` feature.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = synthient::grpcschema::fetch(std::env::var("SYNTHIENT_API_KEY")?).await?;
//! for file in &schema.descriptor_set.file {
//!     println!("{}", file.name());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`Schema::descriptor_set`] can be encoded with
//! [`prost::Message::encode_to_vec`] for tools such as `grpcurl` and `buf`.
//! Unless [`plaintext`](Fetch::plaintext) is set, the connection uses rustls
//! with the platform's trusted certificates, whichever TLS feature the HTTP
//! client uses.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::stream;
use prost::Message;
use prost_types::{FileDescriptorProto, FileDescriptorSet};
use reqwest::Url;
use tonic::metadata::AsciiMetadataValue;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic_reflection::pb::v1::ServerReflectionRequest;
use tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient;
use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;

pub use prost;
pub use prost_types;
pub use tonic::Code;

/// The address of the Synthient gRPC API.
pub const DEFAULT_ENDPOINT: &str = "grpc.synthient.com:443";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MESSAGE_SIZE: usize = 32 << 20;

/// Starts a fetch of the protobuf files that define the server's services,
/// along with all of their imports. `api_key` is sent as `x-api-key`
/// metadata unless it is blank. Await the returned [`Fetch`] to run it,
/// optionally after changing its options.
pub fn fetch(api_key: impl Into<String>) -> Fetch {
    Fetch {
        api_key: api_key.into(),
        endpoint: DEFAULT_ENDPOINT.to_owned(),
        plaintext: false,
        symbols: Vec::new(),
        timeout: DEFAULT_TIMEOUT,
    }
}

/// A schema fetch that runs when awaited. Create one with [`fetch`].
#[must_use = "a schema fetch does nothing until it is awaited"]
pub struct Fetch {
    api_key: String,
    endpoint: String,
    plaintext: bool,
    symbols: Vec<String>,
    timeout: Duration,
}

impl fmt::Debug for Fetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fetch")
            .field("endpoint", &self.endpoint)
            .field("plaintext", &self.plaintext)
            .field("symbols", &self.symbols)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Fetch {
    /// Sets the server as `host:port`, a bare host (port 443 is assumed), or
    /// a `grpc://` or `https://` URL. The default is [`DEFAULT_ENDPOINT`].
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Disables TLS, for local development servers.
    pub fn plaintext(mut self, plaintext: bool) -> Self {
        self.plaintext = plaintext;
        self
    }

    /// Resolves these fully qualified service or message names instead of
    /// every service the server lists. The listing leaves out the
    /// `grpc.reflection` services, so name one here to fetch its schema.
    pub fn symbols<I, S>(mut self, symbols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.symbols = symbols.into_iter().map(Into::into).collect();
        self
    }

    /// Sets how long the whole fetch may take. The default is 30 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn run(self) -> Result<Schema, SchemaError> {
        let (endpoint, host) = normalize_endpoint(&self.endpoint)?;
        let mut reflector = Reflector::connect(&endpoint, host, self.plaintext, &self.api_key)?;

        let mut symbols: Vec<String> = Vec::new();
        for symbol in &self.symbols {
            let symbol = symbol.trim();
            if !symbol.is_empty() && !symbols.iter().any(|seen| seen == symbol) {
                symbols.push(symbol.to_owned());
            }
        }
        if symbols.is_empty() {
            symbols = reflector.list_services().await?;
        }
        if symbols.is_empty() {
            return Err(SchemaError::new("server lists no services"));
        }

        let mut files = BTreeMap::new();
        for symbol in &symbols {
            let request = MessageRequest::FileContainingSymbol(symbol.clone());
            reflector.files(request, &mut files).await?;
        }
        loop {
            let missing = missing_dependencies(&files);
            if missing.is_empty() {
                break;
            }
            for name in missing {
                if files.contains_key(&name) {
                    continue;
                }
                let request = MessageRequest::FileByFilename(name.clone());
                reflector.files(request, &mut files).await?;
                if !files.contains_key(&name) {
                    return Err(SchemaError::new(format!(
                        "reflection did not return dependency {name}"
                    )));
                }
            }
        }

        Ok(Schema {
            endpoint,
            symbols,
            descriptor_set: FileDescriptorSet {
                file: ordered_files(files),
            },
        })
    }
}

impl IntoFuture for Fetch {
    type Output = Result<Schema, SchemaError>;
    type IntoFuture = BoxFuture<'static, Result<Schema, SchemaError>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let timeout = self.timeout;
            tokio::time::timeout(timeout, self.run())
                .await
                .unwrap_or_else(|_| {
                    Err(SchemaError::with_code(
                        Code::DeadlineExceeded,
                        format!("fetching the gRPC schema took longer than {timeout:?}"),
                    ))
                })
        })
    }
}

/// The protobuf schema returned by [`fetch`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Schema {
    /// The normalized address that was queried.
    pub endpoint: String,
    /// The service or message names that were resolved.
    pub symbols: Vec<String>,
    /// The resolved files in dependency order, each file after the files it
    /// imports.
    pub descriptor_set: FileDescriptorSet,
}

/// A failed schema fetch. [`explain`](Self::explain) describes common
/// causes.
#[derive(Debug)]
pub struct SchemaError {
    /// The gRPC status code, or `None` when the failure was not a gRPC
    /// status, such as an invalid endpoint or an unusable reflection reply.
    pub code: Option<Code>,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl SchemaError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
            source: None,
        }
    }

    fn with_code(code: Code, message: String) -> Self {
        Self {
            code: Some(code),
            message,
            source: None,
        }
    }

    fn with_source(message: &str, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            code: None,
            message: message.to_owned(),
            source: Some(Box::new(source)),
        }
    }

    /// Returns a human-readable explanation of a common failure, or `None`.
    pub fn explain(&self) -> Option<&'static str> {
        match self.code? {
            Code::Unauthenticated => Some("Missing or invalid API key."),
            Code::PermissionDenied => {
                Some("This API key does not have permission to inspect gRPC schemas.")
            }
            Code::NotFound => Some(
                "No protobuf schema found for the requested symbol. Fetch without symbols to list the available services.",
            ),
            Code::Unimplemented => Some("The gRPC endpoint does not expose server reflection."),
            Code::Unavailable => Some(
                "Could not reach the gRPC endpoint. Check the address, network access, and TLS settings.",
            ),
            Code::DeadlineExceeded => Some(
                "Timed out reading protobuf schemas from gRPC reflection. Use a longer timeout.",
            ),
            _ => None,
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SchemaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl From<tonic::Status> for SchemaError {
    fn from(status: tonic::Status) -> Self {
        Self {
            code: Some(status.code()),
            message: format!(
                "gRPC reflection failed with {:?}: {}",
                status.code(),
                status.message()
            ),
            source: Some(Box::new(status)),
        }
    }
}

struct Reflector {
    client: ServerReflectionClient<Channel>,
    host: String,
    api_key: Option<AsciiMetadataValue>,
}

impl Reflector {
    /// Returns a reflector for `endpoint`, in `host:port` form, that connects
    /// on its first request.
    fn connect(
        endpoint: &str,
        host: String,
        plaintext: bool,
        api_key: &str,
    ) -> Result<Self, SchemaError> {
        let scheme = if plaintext { "http" } else { "https" };
        let mut channel = Endpoint::from_shared(format!("{scheme}://{endpoint}"))
            .map_err(|err| SchemaError::with_source("invalid gRPC endpoint", err))?;
        if !plaintext {
            let tls = ClientTlsConfig::new()
                .domain_name(&host)
                .with_native_roots();
            channel = channel
                .tls_config(tls)
                .map_err(|err| SchemaError::with_source("could not configure TLS", err))?;
        }

        let api_key = match api_key.trim() {
            "" => None,
            key => {
                let mut value = AsciiMetadataValue::try_from(key)
                    .map_err(|_| SchemaError::new("API key contains invalid characters"))?;
                value.set_sensitive(true);
                Some(value)
            }
        };
        Ok(Self {
            client: ServerReflectionClient::new(channel.connect_lazy())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            host,
            api_key,
        })
    }

    /// Sends one request on its own reflection call, so a server that ends a
    /// call early cannot leave the client waiting for a reply.
    async fn exchange(&mut self, request: MessageRequest) -> Result<MessageResponse, SchemaError> {
        let request = ServerReflectionRequest {
            host: self.host.clone(),
            message_request: Some(request),
        };
        let mut call = tonic::Request::new(stream::iter([request]));
        if let Some(api_key) = &self.api_key {
            call.metadata_mut().insert("x-api-key", api_key.clone());
        }
        let mut responses = self.client.server_reflection_info(call).await?.into_inner();
        let response = responses.message().await?.ok_or_else(|| {
            SchemaError::new("reflection stream ended before returning a response")
        })?;
        match response.message_response {
            Some(MessageResponse::ErrorResponse(err)) => Err(SchemaError::with_code(
                Code::from(err.error_code),
                format!("gRPC reflection failed: {}", err.error_message),
            )),
            Some(response) => Ok(response),
            None => Err(SchemaError::new("reflection response is empty")),
        }
    }

    async fn list_services(&mut self) -> Result<Vec<String>, SchemaError> {
        let MessageResponse::ListServicesResponse(list) = self
            .exchange(MessageRequest::ListServices(String::new()))
            .await?
        else {
            return Err(SchemaError::new("reflection response has no service list"));
        };
        let mut services: Vec<String> = list
            .service
            .into_iter()
            .map(|service| service.name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .filter(|name| !name.starts_with("grpc.reflection."))
            .collect();
        services.sort();
        Ok(services)
    }

    /// Sends a file request and adds the returned files to `files`.
    async fn files(
        &mut self,
        request: MessageRequest,
        files: &mut BTreeMap<String, FileDescriptorProto>,
    ) -> Result<(), SchemaError> {
        let MessageResponse::FileDescriptorResponse(response) = self.exchange(request).await?
        else {
            return Err(SchemaError::new(
                "reflection response has no file descriptors",
            ));
        };
        add_files(files, response.file_descriptor_proto)
    }
}

/// Converts an endpoint to `host:port` form, and returns it with the host
/// name used for TLS verification.
fn normalize_endpoint(raw: &str) -> Result<(String, String), SchemaError> {
    let invalid = || SchemaError::new("invalid gRPC endpoint");
    let raw = match raw.trim() {
        "" => DEFAULT_ENDPOINT,
        raw => raw,
    };
    let url = if raw.contains("://") {
        Url::parse(raw)
    } else if raw.matches(':').count() > 1 && !raw.starts_with('[') {
        Url::parse(&format!("grpc://[{}]", raw.trim_end_matches('/')))
    } else {
        Url::parse(&format!("grpc://{raw}"))
    }
    .map_err(|_| invalid())?;

    let valid = matches!(url.scheme(), "grpc" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.path(), "" | "/")
        && url.query().is_none()
        && url.fragment().is_none()
        && url.port() != Some(0);
    let host = url
        .host_str()
        .filter(|host| valid && !host.is_empty())
        .ok_or_else(invalid)?;
    let port = url.port_or_known_default().unwrap_or(443);
    Ok((
        format!("{host}:{port}"),
        host.trim_matches(['[', ']']).to_owned(),
    ))
}

fn add_files(
    files: &mut BTreeMap<String, FileDescriptorProto>,
    raw: Vec<Vec<u8>>,
) -> Result<(), SchemaError> {
    for data in raw {
        let file = FileDescriptorProto::decode(data.as_slice()).map_err(|err| {
            SchemaError::with_source("reflection returned an invalid file descriptor", err)
        })?;
        if file.name().is_empty() {
            return Err(SchemaError::new(
                "reflection returned a file descriptor without a name",
            ));
        }
        files.entry(file.name().to_owned()).or_insert(file);
    }
    Ok(())
}

fn missing_dependencies(files: &BTreeMap<String, FileDescriptorProto>) -> BTreeSet<String> {
    files
        .values()
        .flat_map(|file| &file.dependency)
        .filter(|dependency| !files.contains_key(*dependency))
        .cloned()
        .collect()
}

/// Orders files so that each comes after the files it imports.
fn ordered_files(mut files: BTreeMap<String, FileDescriptorProto>) -> Vec<FileDescriptorProto> {
    fn visit(
        file: FileDescriptorProto,
        files: &mut BTreeMap<String, FileDescriptorProto>,
        ordered: &mut Vec<FileDescriptorProto>,
    ) {
        let mut dependencies: Vec<&String> = file.dependency.iter().collect();
        dependencies.sort();
        for dependency in dependencies {
            if let Some(dependency) = files.remove(dependency) {
                visit(dependency, files, ordered);
            }
        }
        ordered.push(file);
    }

    let mut ordered = Vec::with_capacity(files.len());
    while let Some((_, file)) = files.pop_first() {
        visit(file, &mut files, &mut ordered);
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_endpoints() {
        let cases = [
            ("", "grpc.synthient.com:443", "grpc.synthient.com"),
            (
                "grpc.synthient.com",
                "grpc.synthient.com:443",
                "grpc.synthient.com",
            ),
            (" localhost:50051/ ", "localhost:50051", "localhost"),
            ("grpc://localhost:50051", "localhost:50051", "localhost"),
            (
                "https://grpc.synthient.com",
                "grpc.synthient.com:443",
                "grpc.synthient.com",
            ),
            (
                "https://grpc.synthient.com:8443/",
                "grpc.synthient.com:8443",
                "grpc.synthient.com",
            ),
            ("[::1]:9000", "[::1]:9000", "::1"),
            ("::1", "[::1]:443", "::1"),
            ("127.0.0.1:50051", "127.0.0.1:50051", "127.0.0.1"),
        ];
        for (raw, endpoint, host) in cases {
            let normalized = normalize_endpoint(raw).unwrap();
            assert_eq!(
                normalized,
                (endpoint.to_owned(), host.to_owned()),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn rejects_invalid_endpoints() {
        let invalid = [
            "grpc://",
            "http://localhost:50051",
            "grpc://user:pass@localhost:50051",
            "localhost:50051/path",
            "localhost:50051?x=1",
            "localhost:0",
            "bad host:50051",
            "localhost:port",
        ];
        for raw in invalid {
            assert!(normalize_endpoint(raw).is_err(), "{raw:?}");
        }
    }

    fn file(name: &str, dependencies: &[&str]) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(name.to_owned()),
            dependency: dependencies
                .iter()
                .map(|dependency| (*dependency).to_owned())
                .collect(),
            ..FileDescriptorProto::default()
        }
    }

    #[test]
    fn orders_files_after_their_imports() {
        let files: BTreeMap<String, FileDescriptorProto> = [
            file("a.proto", &["c.proto", "b.proto"]),
            file("b.proto", &["c.proto"]),
            file("c.proto", &[]),
            file("d.proto", &["missing.proto"]),
        ]
        .into_iter()
        .map(|file| (file.name().to_owned(), file))
        .collect();

        assert_eq!(
            missing_dependencies(&files),
            BTreeSet::from(["missing.proto".to_owned()])
        );
        let names: Vec<String> = ordered_files(files)
            .iter()
            .map(|file| file.name().to_owned())
            .collect();
        assert_eq!(names, ["c.proto", "b.proto", "a.proto", "d.proto"]);
    }

    #[test]
    fn rejects_unusable_file_descriptors() {
        let mut files = BTreeMap::new();

        assert!(add_files(&mut files, vec![vec![0xff, 0xff]]).is_err());
        assert!(add_files(&mut files, vec![file("", &[]).encode_to_vec()]).is_err());

        let first = file("a.proto", &[]);
        let mut second = file("a.proto", &["b.proto"]);
        second.package = Some("other".to_owned());
        add_files(
            &mut files,
            vec![first.encode_to_vec(), second.encode_to_vec()],
        )
        .unwrap();
        assert_eq!(files["a.proto"], first);
    }

    #[test]
    fn explains_common_codes() {
        let explained = [
            Code::Unauthenticated,
            Code::PermissionDenied,
            Code::NotFound,
            Code::Unimplemented,
            Code::Unavailable,
            Code::DeadlineExceeded,
        ];
        for code in explained {
            let err = SchemaError::from(tonic::Status::new(code, "x"));
            assert!(err.explain().is_some(), "{code:?}");
        }
        assert_eq!(
            SchemaError::from(tonic::Status::internal("x")).explain(),
            None
        );
        assert_eq!(SchemaError::new("plain").explain(), None);
    }
}
