use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use rust_reg::pdf_server::v1::{
    template_chunk, CloseRequest, FieldValue, RenderRequest, TemplateChunk, TemplateMeta, ValueRow,
};
use rust_reg::pdf_server::{
    serve_listener, serve_listener_with, PdfToolingClient, PdfToolingService,
};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tonic::Code;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn fs_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

async fn spawn(idle: Duration) -> (PdfToolingClient<tonic::transport::Channel>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = serve_listener_with(listener, PdfToolingService::new(idle)).await;
    });
    let client = PdfToolingClient::connect(format!("http://{addr}"))
        .await
        .unwrap();
    (client, server)
}

async fn stop(server: JoinHandle<()>) {
    server.abort();
    let _ = server.await;
}

fn session_dirs() -> Vec<PathBuf> {
    let root = std::env::temp_dir().join("pdf-tooling");
    let mut dirs = Vec::new();
    if let Ok(read) = std::fs::read_dir(root) {
        for entry in read.flatten() {
            if entry.path().is_dir() {
                dirs.push(entry.path());
            }
        }
    }
    dirs.sort();
    dirs
}

fn meta(filename: &str) -> TemplateChunk {
    TemplateChunk {
        part: Some(template_chunk::Part::Meta(TemplateMeta {
            filename: filename.to_string(),
        })),
    }
}

fn data(bytes: Vec<u8>) -> TemplateChunk {
    TemplateChunk {
        part: Some(template_chunk::Part::Data(bytes)),
    }
}

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (name, bytes) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn card_yaml() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/template/card.yaml");
    std::fs::read_to_string(path)
        .unwrap()
        .replace("../struct-to-pdf/fonts/", "fonts/")
}

fn card_zip() -> Vec<u8> {
    let yaml = card_yaml();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let regular =
        std::fs::read(root.join("struct-to-pdf/fonts/LiberationSerif-Regular.ttf")).unwrap();
    let bold = std::fs::read(root.join("struct-to-pdf/fonts/LiberationSerif-Bold.ttf")).unwrap();
    zip_bytes(&[
        ("card.yaml", yaml.as_bytes()),
        ("fonts/LiberationSerif-Regular.ttf", &regular),
        ("fonts/LiberationSerif-Bold.ttf", &bold),
    ])
}

#[tokio::test]
async fn server_lists_rpcs() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve_listener(listener).await.unwrap();
    });

    let mut client = PdfToolingClient::connect(format!("http://{addr}"))
        .await
        .unwrap();
    let err = client
        .close(CloseRequest {
            session_id: String::new(),
        })
        .await
        .expect_err("unknown session");
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn load_returns_fields() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let before = session_dirs();
    let view = client
        .load_template(tokio_stream::iter(vec![
            meta("card.yaml"),
            data(card_zip()),
        ]))
        .await
        .unwrap()
        .into_inner();
    assert!(!view.session_id.is_empty());
    let names: Vec<&str> = view
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, ["name", "surname", "company name"]);
    for field in &view.fields {
        assert_eq!(field.box_ids, ["card-text"]);
    }
    let created: Vec<PathBuf> = session_dirs()
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(created.len(), 1);
    assert!(created[0].join("card.yaml").is_file());

    let err = client
        .render(RenderRequest {
            session_id: view.session_id,
            rows: Vec::new(),
        })
        .await
        .expect_err("idle time is zero");
    assert_eq!(err.code(), Code::NotFound);
    assert!(!created[0].exists());
    stop(server).await;
}

#[tokio::test]
async fn bad_mustache() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let before = session_dirs();
    let yaml = card_yaml().replace(
        "'<span font=\"font1\">{{name}} {{surname}}</span> <span font=\"font2\">{{company name}}</span>'",
        "'{{}}'",
    );
    let err = client
        .load_template(tokio_stream::iter(vec![
            meta("card.yaml"),
            data(zip_bytes(&[("card.yaml", yaml.as_bytes())])),
        ]))
        .await
        .expect_err("empty field");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("empty"), "{}", err.message());
    assert_eq!(session_dirs(), before);

    let err = client
        .render(RenderRequest {
            session_id: "not-stored".to_string(),
            rows: Vec::new(),
        })
        .await
        .expect_err("no session");
    assert_eq!(err.code(), Code::NotFound);
    stop(server).await;
}

#[tokio::test]
async fn missing_page_entry() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let before = session_dirs();
    let err = client
        .load_template(tokio_stream::iter(vec![
            meta("missing.yaml"),
            data(card_zip()),
        ]))
        .await
        .expect_err("missing page");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("missing entry"), "{}", err.message());
    assert_eq!(session_dirs(), before);
    stop(server).await;
}

#[tokio::test]
async fn filename_escape() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let before = session_dirs();
    let err = client
        .load_template(tokio_stream::iter(vec![
            meta("../card.yaml"),
            data(card_zip()),
        ]))
        .await
        .expect_err("filename escape");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("filename"), "{}", err.message());
    assert_eq!(session_dirs(), before);
    stop(server).await;
}

#[tokio::test]
async fn zip_slip() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let token = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let sentinel = format!("sentinel-{token}.txt");
    let escaped_name = format!("escaped-{token}.txt");
    let slip = format!("../{escaped_name}");
    let archive = zip_bytes(&[
        (sentinel.as_str(), b"sentinel"),
        (slip.as_str(), b"escaped"),
    ]);
    let before = session_dirs();
    let escaped = std::env::temp_dir().join("pdf-tooling").join(&escaped_name);
    let err = client
        .load_template(tokio_stream::iter(vec![meta("card.yaml"), data(archive)]))
        .await
        .expect_err("zip slip");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("escapes"), "{}", err.message());
    assert!(!escaped.exists(), "{}", escaped.display());
    assert_eq!(session_dirs(), before);
    assert!(!tree_has(
        &std::env::temp_dir().join("pdf-tooling"),
        &sentinel
    ));
    stop(server).await;
}

#[tokio::test]
async fn data_before_meta() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let before = session_dirs();
    let err = client
        .load_template(tokio_stream::iter(vec![data(b"zip".to_vec())]))
        .await
        .expect_err("data before meta");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("meta"), "{}", err.message());
    assert_eq!(session_dirs(), before);
    stop(server).await;
}

const LIVE: Duration = Duration::from_secs(60);
const PDF_CHUNK_BYTES: usize = 64 * 1024;

fn row(values: &[(&str, &str)]) -> ValueRow {
    ValueRow {
        values: values
            .iter()
            .map(|(field, value)| FieldValue {
                field: (*field).to_string(),
                value: (*value).to_string(),
            })
            .collect(),
    }
}

async fn load_card(client: &mut PdfToolingClient<tonic::transport::Channel>) -> String {
    client
        .load_template(tokio_stream::iter(vec![
            meta("card.yaml"),
            data(card_zip()),
        ]))
        .await
        .unwrap()
        .into_inner()
        .session_id
}

fn pdf_files() -> Vec<PathBuf> {
    let root = std::env::temp_dir().join("pdf-tooling");
    let mut paths = Vec::new();
    if let Ok(read) = std::fs::read_dir(root) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("pdf") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

async fn render_pdf(
    client: &mut PdfToolingClient<tonic::transport::Channel>,
    session_id: String,
    rows: Vec<ValueRow>,
) -> (PathBuf, Vec<u8>) {
    let response = client
        .render(RenderRequest { session_id, rows })
        .await
        .unwrap()
        .into_inner();
    let path = PathBuf::from(&response.path);
    assert!(path.is_file(), "{}", response.path);
    assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("pdf"));
    let bytes = std::fs::read(&path).unwrap();
    (path, bytes)
}

fn page_count(bytes: &[u8]) -> i64 {
    let doc = lopdf::Document::load_mem(bytes).expect("pdf bytes");
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = doc.get_dictionary(root).unwrap();
    let pages_id = catalog.get(b"Pages").unwrap().as_reference().unwrap();
    let pages = doc.get_dictionary(pages_id).unwrap();
    match pages.get(b"Count").unwrap() {
        lopdf::Object::Integer(count) => *count,
        other => panic!("Count is {other:?}"),
    }
}

fn page_text(bytes: &[u8], page: u32) -> String {
    lopdf::Document::load_mem(bytes)
        .unwrap()
        .extract_text(&[page])
        .unwrap()
        .replace('\n', " ")
}

#[tokio::test]
async fn render_one_page() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = load_card(&mut client).await;
    let (path, pdf) = render_pdf(
        &mut client,
        session_id,
        vec![row(&[
            ("name", "Ann"),
            ("surname", "Lee"),
            ("company name", "North"),
        ])],
    )
    .await;
    assert_eq!(page_count(&pdf), 1);
    let text = page_text(&pdf, 1);
    assert!(text.contains("Ann"), "{text}");
    assert!(text.contains("Lee"), "{text}");
    assert!(text.contains("North"), "{text}");
    assert!(!text.contains("{{"), "{text}");
    let _ = std::fs::remove_file(path);
    stop(server).await;
}

#[tokio::test]
async fn render_two_pages() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = load_card(&mut client).await;
    let (path, pdf) = render_pdf(
        &mut client,
        session_id,
        vec![
            row(&[
                ("name", "Ann"),
                ("surname", "Lee"),
                ("company name", "North"),
            ]),
            row(&[
                ("name", "Bo"),
                ("surname", "Kim"),
                ("company name", "South"),
            ]),
        ],
    )
    .await;
    assert_eq!(page_count(&pdf), 2);
    let first = page_text(&pdf, 1);
    let second = page_text(&pdf, 2);
    assert!(
        first.contains("Ann") && first.contains("Lee") && first.contains("North"),
        "{first}"
    );
    assert!(!first.contains("Bo"), "{first}");
    assert!(
        second.contains("Bo") && second.contains("Kim") && second.contains("South"),
        "{second}"
    );
    assert!(!second.contains("Ann"), "{second}");
    let _ = std::fs::remove_file(path);
    stop(server).await;
}

#[tokio::test]
async fn render_chunks() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = load_card(&mut client).await;
    let before = pdf_files();
    let mut stream = client
        .render_chunks(RenderRequest {
            session_id,
            rows: vec![
                row(&[
                    ("name", "Ann"),
                    ("surname", "Lee"),
                    ("company name", "North"),
                ]),
                row(&[
                    ("name", "Bo"),
                    ("surname", "Kim"),
                    ("company name", "South"),
                ]),
            ],
        })
        .await
        .unwrap()
        .into_inner();
    let mut pdf = Vec::new();
    while let Some(chunk) = stream.message().await.unwrap() {
        assert!(chunk.data.len() <= PDF_CHUNK_BYTES);
        pdf.extend(chunk.data);
    }
    assert_eq!(pdf_files(), before);
    assert_eq!(page_count(&pdf), 2);
    assert!(page_text(&pdf, 1).contains("Ann"));
    assert!(page_text(&pdf, 2).contains("Bo"));
    stop(server).await;
}

#[tokio::test]
async fn unknown_field() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = load_card(&mut client).await;
    let before = pdf_files();
    let err = client
        .render(RenderRequest {
            session_id,
            rows: vec![row(&[("nope", "x")])],
        })
        .await
        .expect_err("unknown field");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("nope"), "{}", err.message());
    assert_eq!(pdf_files(), before);
    stop(server).await;
}

#[tokio::test]
async fn duplicate_field() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = load_card(&mut client).await;
    let err = client
        .render(RenderRequest {
            session_id,
            rows: vec![ValueRow {
                values: vec![
                    FieldValue {
                        field: "name".to_string(),
                        value: "Ann".to_string(),
                    },
                    FieldValue {
                        field: "name".to_string(),
                        value: "Bo".to_string(),
                    },
                ],
            }],
        })
        .await
        .expect_err("duplicate field");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("duplicate"), "{}", err.message());
    stop(server).await;
}

#[tokio::test]
async fn empty_rows() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = load_card(&mut client).await;
    let err = client
        .render(RenderRequest {
            session_id,
            rows: Vec::new(),
        })
        .await
        .expect_err("no rows");
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("no rows"), "{}", err.message());
    stop(server).await;
}

#[tokio::test]
async fn unknown_session() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let session_id = "11111111-1111-1111-1111-111111111111".to_string();
    let render_err = client
        .render(RenderRequest {
            session_id: session_id.clone(),
            rows: vec![row(&[("name", "Ann")])],
        })
        .await
        .expect_err("unknown render session");
    assert_eq!(render_err.code(), Code::NotFound);
    let close_err = client
        .close(CloseRequest { session_id })
        .await
        .expect_err("unknown close session");
    assert_eq!(close_err.code(), Code::NotFound);
    stop(server).await;
}

#[tokio::test]
async fn close_then_render() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(LIVE).await;
    let before = session_dirs();
    let session_id = load_card(&mut client).await;
    let created: Vec<PathBuf> = session_dirs()
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(created.len(), 1);
    client
        .close(CloseRequest {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    assert!(!created[0].exists());
    let render_err = client
        .render(RenderRequest {
            session_id: session_id.clone(),
            rows: vec![row(&[
                ("name", "Ann"),
                ("surname", "Lee"),
                ("company name", "North"),
            ])],
        })
        .await
        .expect_err("closed session");
    assert_eq!(render_err.code(), Code::NotFound);
    let close_err = client
        .close(CloseRequest { session_id })
        .await
        .expect_err("second close");
    assert_eq!(close_err.code(), Code::NotFound);
    stop(server).await;
}

#[tokio::test]
async fn expired_session() {
    let _guard = fs_lock().lock().await;
    let (mut client, server) = spawn(Duration::ZERO).await;
    let before = session_dirs();
    let session_id = load_card(&mut client).await;
    let created: Vec<PathBuf> = session_dirs()
        .into_iter()
        .filter(|dir| !before.contains(dir))
        .collect();
    assert_eq!(created.len(), 1);
    let err = client
        .render(RenderRequest {
            session_id,
            rows: vec![row(&[
                ("name", "Ann"),
                ("surname", "Lee"),
                ("company name", "North"),
            ])],
        })
        .await
        .expect_err("expired session");
    assert_eq!(err.code(), Code::NotFound);
    assert!(!created[0].exists());
    stop(server).await;
}

fn tree_has(root: &std::path::Path, name: &str) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|item| item.to_str()) == Some(name) {
                return true;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    false
}
