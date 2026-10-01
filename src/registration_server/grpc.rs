//! Registration gRPC service. Compiled for tests only.
//!
//! Venue clients are browsers. They call the HTTP routes and decode JSON
//! with the engine's `JSON.parse`. Shipping this service in the device
//! binary would add the generated stub, reflection, and a second accept
//! loop for the same lookup and print calls.

use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::registration_server::print_queue::{EnqueueError, PrintJob};
use crate::registration_server::App;

pub mod v1 {
    tonic::include_proto!("irbis.registration.v1");
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("registration_descriptor");
}

use v1::registration_server::{Registration, RegistrationServer};
use v1::{EnqueuePrintRequest, EnqueuePrintResponse, LookupRequest, LookupResponse};

pub async fn serve(
    listener: TcpListener,
    app: App,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), tonic::transport::Error> {
    let incoming = TcpListenerStream::new(listener);
    let signal = async move {
        loop {
            if shutdown.changed().await.is_err() || *shutdown.borrow() {
                break;
            }
        }
    };
    router(app)
        .serve_with_incoming_shutdown(incoming, signal)
        .await
}

fn router(app: App) -> tonic::transport::server::Router {
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(v1::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("registration reflection descriptor");
    Server::builder()
        .add_service(reflection)
        .add_service(RegistrationServer::new(RegistrationService { app }))
}

struct RegistrationService {
    app: App,
}

#[tonic::async_trait]
impl Registration for RegistrationService {
    async fn lookup(
        &self,
        request: Request<LookupRequest>,
    ) -> Result<Response<LookupResponse>, Status> {
        let _guard = self.app.gate.enter_request();
        let id = request.into_inner().id;
        let found = self.app.store.lookup(&id);
        Ok(Response::new(LookupResponse {
            found: found.is_some(),
            id: found.as_ref().map(|row| row.id.clone()).unwrap_or(id),
            name: found.map(|row| row.name).unwrap_or_default(),
        }))
    }

    async fn enqueue_print(
        &self,
        request: Request<EnqueuePrintRequest>,
    ) -> Result<Response<EnqueuePrintResponse>, Status> {
        let _guard = self.app.gate.enter_request();
        let request = request.into_inner();
        let id = request.id;
        self.app
            .prints
            .enqueue(PrintJob {
                id: id.clone(),
                pages: request.pages,
            })
            .map_err(enqueue_status)?;
        Ok(Response::new(EnqueuePrintResponse { id }))
    }
}

fn enqueue_status(err: EnqueueError) -> Status {
    match err {
        EnqueueError::EmptyId => Status::invalid_argument("id is empty"),
        EnqueueError::NoPages => Status::invalid_argument("pages is 0"),
        EnqueueError::Full => Status::resource_exhausted("print queue is full"),
        EnqueueError::Closed => Status::unavailable("print queue is closed"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;
    use tokio::sync::watch;

    use super::v1::registration_client::RegistrationClient;
    use super::v1::{EnqueuePrintRequest, LookupRequest};
    use super::serve;
    use crate::registration_server::{App, Registration};

    #[tokio::test]
    async fn lookup_and_print_over_grpc() {
        let (app, _worker) = App::new();
        app.store.write_batch(vec![Registration {
            id: "1".into(),
            name: "Ann".into(),
        }]);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server = tokio::spawn(serve(listener, app, shutdown_rx));

        let mut client = RegistrationClient::connect(format!("http://{addr}"))
            .await
            .unwrap();
        let lookup = client
            .lookup(LookupRequest { id: "1".into() })
            .await
            .unwrap()
            .into_inner();
        assert!(lookup.found);
        assert_eq!(lookup.name, "Ann");
        let print = client
            .enqueue_print(EnqueuePrintRequest {
                id: "badge".into(),
                pages: 2,
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(print.id, "badge");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
