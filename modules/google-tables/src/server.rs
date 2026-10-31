//! gRPC surface. Setup calls use the in-memory session. `GetWatch` is `NOT_FOUND` until a watch exists.

use std::net::SocketAddr;
use std::pin::Pin;

use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::irbis::google_tables::v1::google_tables_server::{GoogleTables, GoogleTablesServer};
use crate::irbis::google_tables::v1::{
    CloseRequest, CloseResponse, EnsureIdentityRequest, GetSnapshotRequest, GetWatchRequest,
    ListSnapshotsRequest, ListSnapshotsResponse, OpenSpreadsheetRequest, ReadStoredRequest,
    ReadTableRequest, RowBatch, RowChange, SelectColumnsRequest, SelectHeaderRequest,
    SelectSheetRequest, SessionView, SetColumnOptionsRequest, SnapshotBatch, StartWatchRequest,
    StopWatchRequest, StoredBatch, SubscribeRequest, SyncNowRequest, SyncReport, Watch,
};
use crate::irbis::table::v1::Table;
pub use crate::session::GoogleTablesService;

pub const DEFAULT_ADDR: &str = "0.0.0.0:50052";

pub async fn serve(addr: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(addr).await?;
    let service = GoogleTablesService::open("data/google-tables.lmdb")?;
    Server::builder()
        .add_service(GoogleTablesServer::new(service))
        .serve_with_incoming(TcpListenerStream::new(listener))
        .await?;
    Ok(())
}

pub async fn serve_listener(
    listener: TcpListener,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Server::builder()
        .add_service(GoogleTablesServer::new(GoogleTablesService::new()))
        .serve_with_incoming(TcpListenerStream::new(listener))
        .await?;
    Ok(())
}

type RowStream = Pin<Box<dyn futures::Stream<Item = Result<RowBatch, Status>> + Send>>;
type ChangeStream = Pin<Box<dyn futures::Stream<Item = Result<RowChange, Status>> + Send>>;
type StoredStream = Pin<Box<dyn futures::Stream<Item = Result<StoredBatch, Status>> + Send>>;
type SnapshotStream = Pin<Box<dyn futures::Stream<Item = Result<SnapshotBatch, Status>> + Send>>;

#[tonic::async_trait]
impl GoogleTables for GoogleTablesService {
    async fn open_spreadsheet(
        &self,
        request: Request<OpenSpreadsheetRequest>,
    ) -> Result<Response<SessionView>, Status> {
        self.open_spreadsheet(request.into_inner())
            .map(Response::new)
    }

    async fn select_sheet(
        &self,
        request: Request<SelectSheetRequest>,
    ) -> Result<Response<SessionView>, Status> {
        self.select_sheet(request.into_inner()).map(Response::new)
    }

    async fn select_header(
        &self,
        request: Request<SelectHeaderRequest>,
    ) -> Result<Response<Table>, Status> {
        self.select_header(request.into_inner()).map(Response::new)
    }

    async fn select_columns(
        &self,
        request: Request<SelectColumnsRequest>,
    ) -> Result<Response<Table>, Status> {
        self.select_columns(request.into_inner()).map(Response::new)
    }

    async fn set_column_options(
        &self,
        request: Request<SetColumnOptionsRequest>,
    ) -> Result<Response<Table>, Status> {
        self.set_column_options(request.into_inner())
            .map(Response::new)
    }

    async fn ensure_identity(
        &self,
        request: Request<EnsureIdentityRequest>,
    ) -> Result<Response<Table>, Status> {
        self.ensure_identity(request.into_inner())
            .map(Response::new)
    }

    type ReadTableStream = RowStream;

    async fn read_table(
        &self,
        request: Request<ReadTableRequest>,
    ) -> Result<Response<Self::ReadTableStream>, Status> {
        let batches = GoogleTablesService::read_table(self, request.into_inner())?;
        let stream = tokio_stream::iter(batches.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn close(
        &self,
        request: Request<CloseRequest>,
    ) -> Result<Response<CloseResponse>, Status> {
        self.close(request.into_inner()).map(Response::new)
    }

    async fn start_watch(
        &self,
        request: Request<StartWatchRequest>,
    ) -> Result<Response<Watch>, Status> {
        self.start_watch(request.into_inner()).map(Response::new)
    }

    async fn stop_watch(
        &self,
        request: Request<StopWatchRequest>,
    ) -> Result<Response<Watch>, Status> {
        self.stop_watch(request.into_inner()).map(Response::new)
    }

    async fn sync_now(
        &self,
        request: Request<SyncNowRequest>,
    ) -> Result<Response<SyncReport>, Status> {
        self.sync_now(request.into_inner()).await.map(Response::new)
    }

    async fn get_watch(
        &self,
        request: Request<GetWatchRequest>,
    ) -> Result<Response<Watch>, Status> {
        self.get_watch(request.into_inner()).map(Response::new)
    }

    type SubscribeStream = ChangeStream;

    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        self.subscribe(request.into_inner()).map(Response::new)
    }

    type ReadStoredStream = StoredStream;

    async fn read_stored(
        &self,
        request: Request<ReadStoredRequest>,
    ) -> Result<Response<Self::ReadStoredStream>, Status> {
        let batches = self.read_stored_batches(request.into_inner())?;
        let stream = tokio_stream::iter(batches.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn list_snapshots(
        &self,
        request: Request<ListSnapshotsRequest>,
    ) -> Result<Response<ListSnapshotsResponse>, Status> {
        let snapshots = self.list_snapshots(request.into_inner())?;
        Ok(Response::new(ListSnapshotsResponse { snapshots }))
    }

    type GetSnapshotStream = SnapshotStream;

    async fn get_snapshot(
        &self,
        request: Request<GetSnapshotRequest>,
    ) -> Result<Response<Self::GetSnapshotStream>, Status> {
        let batches = self.get_snapshot_batches(request.into_inner())?;
        let stream = tokio_stream::iter(batches.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }
}
