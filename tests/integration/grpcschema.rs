use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{self, BoxStream};
use synthient::grpcschema::{self, Code};
use tokio::net::TcpListener;
use tonic::service::InterceptorLayer;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Status, Streaming};
use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;
use tonic_reflection::pb::v1::server_reflection_server::{
    ServerReflection, ServerReflectionServer,
};
use tonic_reflection::pb::v1::{
    ErrorResponse, FileDescriptorResponse, ListServiceResponse, ServerReflectionRequest,
    ServerReflectionResponse, ServiceResponse,
};

use synthient::grpcschema::prost::Message;
use synthient::grpcschema::prost_types::FileDescriptorProto;

use crate::server::closed_addr;

fn local(addr: &str) -> grpcschema::Fetch {
    grpcschema::fetch("test-key").endpoint(addr).plaintext(true)
}

/// Returns the server's address and the x-api-key metadata of every call.
async fn start_server() -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let keys = Arc::new(Mutex::new(Vec::new()));
    let seen = keys.clone();

    let (_, health) = tonic_health::server::health_reporter();
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();
    let interceptor = move |request: tonic::Request<()>| {
        let key = request.metadata().get("x-api-key");
        seen.lock()
            .unwrap()
            .push(key.map(|key| key.to_str().unwrap().to_owned()));
        Ok(request)
    };
    tokio::spawn(
        Server::builder()
            .layer(InterceptorLayer::new(interceptor))
            .add_service(health)
            .add_service(reflection)
            .serve_with_incoming(TcpIncoming::from(listener)),
    );
    (addr, keys)
}

type Reply = fn(&ServerReflectionRequest) -> Option<MessageResponse>;

/// A reflection service that answers the first request of each call with a
/// scripted reply, or ends the call without one.
struct Scripted(Reply);

#[tonic::async_trait]
impl ServerReflection for Scripted {
    type ServerReflectionInfoStream = BoxStream<'static, Result<ServerReflectionResponse, Status>>;

    async fn server_reflection_info(
        &self,
        request: tonic::Request<Streaming<ServerReflectionRequest>>,
    ) -> Result<tonic::Response<Self::ServerReflectionInfoStream>, Status> {
        let first = request.into_inner().message().await?.unwrap();
        let reply = (self.0)(&first).map(|message| {
            Ok(ServerReflectionResponse {
                message_response: Some(message),
                ..ServerReflectionResponse::default()
            })
        });
        Ok(tonic::Response::new(Box::pin(stream::iter(reply))))
    }
}

async fn start_scripted(reply: Reply) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(
        Server::builder()
            .add_service(ServerReflectionServer::new(Scripted(reply)))
            .serve_with_incoming(TcpIncoming::from(listener)),
    );
    addr
}

fn services(names: &[&str]) -> Option<MessageResponse> {
    let service = names
        .iter()
        .map(|name| ServiceResponse {
            name: (*name).to_owned(),
        })
        .collect();
    Some(MessageResponse::ListServicesResponse(ListServiceResponse {
        service,
    }))
}

fn files(files: &[(&str, &[&str])]) -> Option<MessageResponse> {
    let file_descriptor_proto = files
        .iter()
        .map(|(name, dependencies)| {
            FileDescriptorProto {
                name: Some((*name).to_owned()),
                dependency: dependencies
                    .iter()
                    .map(|dependency| (*dependency).to_owned())
                    .collect(),
                ..FileDescriptorProto::default()
            }
            .encode_to_vec()
        })
        .collect();
    Some(MessageResponse::FileDescriptorResponse(
        FileDescriptorResponse {
            file_descriptor_proto,
        },
    ))
}

#[tokio::test]
async fn fetches_every_service() {
    let (addr, keys) = start_server().await;

    let schema = local(&addr).await.unwrap();

    assert_eq!(schema.endpoint, addr);
    assert_eq!(schema.symbols, ["grpc.health.v1.Health"]);
    let health = schema
        .descriptor_set
        .file
        .iter()
        .find(|file| file.package() == "grpc.health.v1")
        .unwrap();
    assert_eq!(health.service[0].name(), "Health");
    let keys = keys.lock().unwrap();
    assert!(keys.len() >= 2, "{keys:?}");
    assert!(
        keys.iter().all(|key| key.as_deref() == Some("test-key")),
        "{keys:?}"
    );
}

#[tokio::test]
async fn applies_the_options() {
    let (addr, keys) = start_server().await;

    let schema = grpcschema::fetch(" ")
        .endpoint(format!("grpc://{addr}"))
        .plaintext(true)
        .symbols(["grpc.reflection.v1.ServerReflection"])
        .await
        .unwrap();

    assert!(
        schema
            .descriptor_set
            .file
            .iter()
            .any(|file| file.package() == "grpc.reflection.v1")
    );
    assert!(keys.lock().unwrap().iter().all(Option::is_none));

    let schema = local(&addr)
        .symbols([
            " grpc.health.v1.HealthCheckRequest ",
            "grpc.health.v1.HealthCheckRequest",
            "",
        ])
        .await
        .unwrap();

    assert_eq!(schema.symbols, ["grpc.health.v1.HealthCheckRequest"]);
}

#[tokio::test]
async fn orders_files_after_their_imports() {
    let addr = start_scripted(|request| match &request.message_request {
        Some(MessageRequest::ListServices(_)) => services(&["test.v1.Service"]),
        Some(MessageRequest::FileContainingSymbol(_)) => {
            files(&[("service.proto", &["types.proto"])])
        }
        Some(MessageRequest::FileByFilename(name)) if name == "types.proto" => {
            files(&[("types.proto", &["base.proto"])])
        }
        _ => files(&[("base.proto", &[])]),
    })
    .await;

    let schema = local(&addr).await.unwrap();

    let names: Vec<&str> = schema
        .descriptor_set
        .file
        .iter()
        .map(|file| file.name())
        .collect();
    assert_eq!(names, ["base.proto", "types.proto", "service.proto"]);
}

#[tokio::test]
async fn an_unknown_symbol_is_not_found() {
    let (addr, _) = start_server().await;

    let err = local(&addr)
        .symbols(["synthient.v1.Missing"])
        .await
        .unwrap_err();

    assert_eq!(err.code, Some(Code::NotFound));
    assert!(err.explain().is_some());
}

#[tokio::test]
async fn reflection_error_replies_keep_their_code() {
    let addr = start_scripted(|_| {
        Some(MessageResponse::ErrorResponse(ErrorResponse {
            error_code: Code::PermissionDenied as i32,
            error_message: "denied".to_owned(),
        }))
    })
    .await;

    let err = local(&addr).await.unwrap_err();

    assert_eq!(err.code, Some(Code::PermissionDenied));
    assert!(err.to_string().contains("denied"));
}

#[tokio::test]
async fn unusable_replies_are_errors() {
    let cases: [(Reply, &str); 4] = [
        (|_| None, "ended before returning a response"),
        (|_| files(&[]), "no service list"),
        (|_| services(&[]), "lists no services"),
        (
            |request| match request.message_request {
                Some(MessageRequest::ListServices(_)) => services(&["test.v1.Service"]),
                Some(MessageRequest::FileContainingSymbol(_)) => {
                    files(&[("a.proto", &["b.proto"])])
                }
                _ => files(&[("c.proto", &[])]),
            },
            "did not return dependency b.proto",
        ),
    ];
    for (reply, message) in cases {
        let addr = start_scripted(reply).await;

        let err = local(&addr).await.unwrap_err();

        assert_eq!(err.code, None, "{err}");
        assert!(err.to_string().contains(message), "{err}");
    }
}

#[tokio::test]
async fn an_unreachable_endpoint_is_unavailable() {
    let err = local(&closed_addr().await).await.unwrap_err();

    assert_eq!(err.code, Some(Code::Unavailable), "{err}");
    assert!(err.explain().is_some());
}

#[tokio::test]
async fn a_silent_server_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            open.push(socket);
        }
    });

    let err = local(&addr)
        .timeout(Duration::from_millis(200))
        .await
        .unwrap_err();

    assert_eq!(err.code, Some(Code::DeadlineExceeded), "{err}");
}

#[tokio::test]
async fn invalid_endpoints_are_rejected_before_connecting() {
    let err = grpcschema::fetch("key")
        .endpoint("grpc://")
        .await
        .unwrap_err();

    assert_eq!(err.code, None);
    assert_eq!(err.to_string(), "invalid gRPC endpoint");
}

#[test]
fn fetch_futures_are_send() {
    fn send_static<T: Send + 'static>(_: &T) {}
    send_static(&std::future::IntoFuture::into_future(grpcschema::fetch(
        "key",
    )));
    assert!(!format!("{:?}", grpcschema::fetch("secret-key")).contains("secret-key"));
}
