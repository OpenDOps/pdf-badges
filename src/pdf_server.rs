use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Request, Response, Status, Streaming};
use zip::ZipArchive;

use crate::struct_to_pdf::{index_template, load_page, render_rows, Page, TemplateIndex};

pub mod v1 {
    tonic::include_proto!("irbis.pdf.v1");
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("pdf_descriptor");
}

use v1::pdf_tooling_server::{PdfTooling, PdfToolingServer};
use v1::{
    CloseRequest, CloseResponse, PdfChunk, RenderRequest, RenderResponse, TemplateChunk,
    TemplateView,
};

pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
pub const IDLE_LIMIT: Duration = Duration::from_secs(15 * 60);

const MAX_UNCOMPRESSED_BYTES: u64 = MAX_MESSAGE_BYTES as u64;
const PDF_CHUNK_BYTES: usize = 64 * 1024;

struct Session {
    directory: PathBuf,
    base_dir: PathBuf,
    page: Page,
    index: TemplateIndex,
    last_used: Instant,
}

struct Sessions {
    idle: Duration,
    open: Mutex<HashMap<String, Session>>,
}

impl Drop for Sessions {
    fn drop(&mut self) {
        let mut open = self.open.lock().unwrap_or_else(|err| err.into_inner());
        for (_, session) in open.drain() {
            let _ = std::fs::remove_dir_all(session.directory);
        }
    }
}

#[derive(Clone)]
pub struct PdfToolingService {
    sessions: Arc<Sessions>,
}

impl PdfToolingService {
    pub fn new(idle: Duration) -> Self {
        Self {
            sessions: Arc::new(Sessions {
                idle,
                open: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn expire(&self) {
        let mut open = self
            .sessions
            .open
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        expire_locked(&mut open, self.sessions.idle);
    }
}

fn expire_locked(open: &mut HashMap<String, Session>, idle: Duration) {
    let now = Instant::now();
    let stale: Vec<String> = open
        .iter()
        .filter(|(_, session)| now.duration_since(session.last_used) >= idle)
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        if let Some(session) = open.remove(&id) {
            let _ = std::fs::remove_dir_all(session.directory);
        }
    }
}

#[tonic::async_trait]
impl PdfTooling for PdfToolingService {
    async fn load_template(
        &self,
        request: Request<Streaming<TemplateChunk>>,
    ) -> Result<Response<TemplateView>, Status> {
        self.expire();
        let mut inbound = request.into_inner();
        let (filename, bytes) = read_upload(&mut inbound).await?;
        let view = install_template(&self.sessions, &filename, bytes)?;
        Ok(Response::new(view))
    }

    async fn render(
        &self,
        request: Request<RenderRequest>,
    ) -> Result<Response<RenderResponse>, Status> {
        let pdf = self.draw(request.into_inner())?;
        let path = write_pdf(&pdf)?;
        Ok(Response::new(RenderResponse { path }))
    }

    type RenderChunksStream = futures::stream::Iter<std::vec::IntoIter<Result<PdfChunk, Status>>>;

    async fn render_chunks(
        &self,
        request: Request<RenderRequest>,
    ) -> Result<Response<Self::RenderChunksStream>, Status> {
        let pdf = self.draw(request.into_inner())?;
        let chunks = pdf
            .chunks(PDF_CHUNK_BYTES)
            .map(|chunk| {
                Ok(PdfChunk {
                    data: chunk.to_vec(),
                })
            })
            .collect::<Vec<_>>();
        Ok(Response::new(futures::stream::iter(chunks)))
    }

    async fn close(
        &self,
        request: Request<CloseRequest>,
    ) -> Result<Response<CloseResponse>, Status> {
        let session_id = request.into_inner().session_id;
        let mut open = self
            .sessions
            .open
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        expire_locked(&mut open, self.sessions.idle);
        let Some(session) = open.remove(&session_id) else {
            return Err(Status::not_found(format!("session {session_id}")));
        };
        let _ = std::fs::remove_dir_all(session.directory);
        Ok(Response::new(CloseResponse {}))
    }
}

impl PdfToolingService {
    fn draw(&self, request: RenderRequest) -> Result<Vec<u8>, Status> {
        let (page, index, base_dir) = {
            let mut open = self
                .sessions
                .open
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            expire_locked(&mut open, self.sessions.idle);
            let Some(session) = open.get_mut(&request.session_id) else {
                return Err(Status::not_found(format!("session {}", request.session_id)));
            };
            session.last_used = Instant::now();
            (
                session.page.clone(),
                session.index.clone(),
                session.base_dir.clone(),
            )
        };
        let rows = value_rows(&index, &request.rows)?;
        render_rows(&page, &index, &rows, &base_dir).map_err(invalid_render)
    }
}

fn write_pdf(bytes: &[u8]) -> Result<String, Status> {
    let root = std::env::temp_dir().join("pdf-tooling");
    std::fs::create_dir_all(&root).map_err(|err| Status::internal(err.to_string()))?;
    let path = root.join(format!("{}.pdf", uuid::Uuid::new_v4()));
    if let Err(err) = std::fs::write(&path, bytes) {
        let _ = std::fs::remove_file(&path);
        return Err(Status::internal(err.to_string()));
    }
    Ok(path.to_string_lossy().into_owned())
}

fn value_rows(
    index: &TemplateIndex,
    rows: &[v1::ValueRow],
) -> Result<Vec<HashMap<String, String>>, Status> {
    if rows.is_empty() {
        return Err(Status::invalid_argument("no rows"));
    }
    let mut maps = Vec::with_capacity(rows.len());
    for row in rows {
        let mut map = HashMap::new();
        for value in &row.values {
            if map.contains_key(&value.field) {
                return Err(Status::invalid_argument(format!(
                    "duplicate field {}",
                    value.field
                )));
            }
            if index.fields.iter().all(|field| field.name != value.field) {
                return Err(Status::invalid_argument(format!(
                    "unknown field {}",
                    value.field
                )));
            }
            map.insert(value.field.clone(), value.value.clone());
        }
        maps.push(map);
    }
    Ok(maps)
}

async fn read_upload(inbound: &mut Streaming<TemplateChunk>) -> Result<(String, Vec<u8>), Status> {
    let first = inbound
        .message()
        .await?
        .ok_or_else(|| Status::invalid_argument("missing meta"))?;
    let filename = match first.part {
        Some(v1::template_chunk::Part::Meta(meta)) => meta.filename,
        Some(v1::template_chunk::Part::Data(_)) => {
            return Err(Status::invalid_argument("first message must be meta"));
        }
        None => return Err(Status::invalid_argument("missing meta")),
    };
    if !single_component(&filename) {
        return Err(Status::invalid_argument(
            "filename must be a single path component",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = inbound.message().await? {
        match chunk.part {
            Some(v1::template_chunk::Part::Data(data)) => bytes.extend(data),
            _ => return Err(Status::invalid_argument("expected zip bytes")),
        }
    }
    Ok((filename, bytes))
}

fn single_component(filename: &str) -> bool {
    if filename.is_empty() || filename.contains('/') || filename.contains('\\') {
        return false;
    }
    match Path::new(filename)
        .components()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [Component::Normal(part)] => *part != "." && *part != "..",
        _ => false,
    }
}

fn install_template(
    sessions: &Sessions,
    filename: &str,
    bytes: Vec<u8>,
) -> Result<TemplateView, Status> {
    let root = std::env::temp_dir().join("pdf-tooling");
    std::fs::create_dir_all(&root).map_err(|err| Status::internal(err.to_string()))?;
    let directory = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&directory).map_err(|err| Status::internal(err.to_string()))?;
    match prepare_session(&directory, filename, &bytes) {
        Ok((page, index, base_dir)) => {
            let fields = index
                .fields
                .iter()
                .map(|field| v1::TemplateField {
                    name: field.name.clone(),
                    box_ids: field.box_ids.clone(),
                })
                .collect();
            let session_id = uuid::Uuid::new_v4().to_string();
            sessions
                .open
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(
                    session_id.clone(),
                    Session {
                        directory,
                        base_dir,
                        page,
                        index,
                        last_used: Instant::now(),
                    },
                );
            Ok(TemplateView { session_id, fields })
        }
        Err(status) => {
            let _ = std::fs::remove_dir_all(&directory);
            Err(status)
        }
    }
}

fn prepare_session(
    directory: &Path,
    filename: &str,
    bytes: &[u8],
) -> Result<(Page, TemplateIndex, PathBuf), Status> {
    unzip(directory, bytes)?;
    let page_path = directory.join(filename);
    if !page_path.is_file() {
        return Err(Status::invalid_argument(format!(
            "missing entry {filename}"
        )));
    }
    let page = load_page(&page_path).map_err(invalid_render)?;
    let index = index_template(&page).map_err(invalid_render)?;
    let base_dir = page_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| directory.to_path_buf());
    Ok((page, index, base_dir))
}

fn invalid_render(err: crate::struct_to_pdf::RenderError) -> Status {
    Status::invalid_argument(err.to_string())
}

fn unzip(directory: &Path, bytes: &[u8]) -> Result<(), Status> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|err| Status::invalid_argument(format!("zip: {err}")))?;
    let mut declared = 0u64;
    let mut actual = 0u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|err| Status::invalid_argument(format!("zip: {err}")))?;
        let size = file.size();
        declared = declared.saturating_add(size);
        if declared > MAX_UNCOMPRESSED_BYTES {
            return Err(Status::invalid_argument("uncompressed size exceeds 64 MiB"));
        }
        let name = file.name().replace('\\', "/");
        let dest = entry_path(directory, &name)?;
        if file.is_dir() || name.ends_with('/') {
            std::fs::create_dir_all(&dest).map_err(|err| Status::internal(err.to_string()))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|err| Status::internal(err.to_string()))?;
        }
        let mut buf = Vec::new();
        file.take(MAX_UNCOMPRESSED_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|err| Status::invalid_argument(format!("zip: {err}")))?;
        actual = actual.saturating_add(buf.len() as u64);
        if actual > MAX_UNCOMPRESSED_BYTES {
            return Err(Status::invalid_argument("uncompressed size exceeds 64 MiB"));
        }
        std::fs::write(&dest, buf).map_err(|err| Status::internal(err.to_string()))?;
    }
    Ok(())
}

fn entry_path(root: &Path, name: &str) -> Result<PathBuf, Status> {
    if name.contains('\0') {
        return Err(Status::invalid_argument(
            "zip entry escapes session directory",
        ));
    }
    let mut dest = root.to_path_buf();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => dest.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if dest == root || !dest.starts_with(root) {
                    return Err(Status::invalid_argument(
                        "zip entry escapes session directory",
                    ));
                }
                dest.pop();
            }
            _ => {
                return Err(Status::invalid_argument(
                    "zip entry escapes session directory",
                ));
            }
        }
    }
    if dest != root && !dest.starts_with(root) {
        return Err(Status::invalid_argument(
            "zip entry escapes session directory",
        ));
    }
    Ok(dest)
}

pub async fn serve(listen: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(listen).await?;
    let addr = listener.local_addr()?;
    eprintln!("pdf-tooling listening on {addr}");
    let incoming = TcpListenerStream::new(listener);
    tokio::select! {
        result = router(PdfToolingService::new(IDLE_LIMIT)).serve_with_incoming(incoming) => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
}

pub async fn serve_listener(listener: TcpListener) -> Result<(), tonic::transport::Error> {
    serve_listener_with(listener, PdfToolingService::new(IDLE_LIMIT)).await
}

pub async fn serve_listener_with(
    listener: TcpListener,
    service: PdfToolingService,
) -> Result<(), tonic::transport::Error> {
    let incoming = TcpListenerStream::new(listener);
    router(service).serve_with_incoming(incoming).await
}

fn router(service: PdfToolingService) -> tonic::transport::server::Router {
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(v1::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("pdf reflection descriptor");
    let pdf = PdfToolingServer::new(service)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES);
    Server::builder().add_service(reflection).add_service(pdf)
}

pub use v1::pdf_tooling_client::PdfToolingClient;
