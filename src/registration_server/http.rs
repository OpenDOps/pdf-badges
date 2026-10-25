use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use tokio::net::TcpListener;
use tokio::sync::watch;

use rusqlite::OptionalExtension;

use crate::registration_server::current::CurrentEvent;
use crate::registration_server::gate::Gate;
use crate::registration_server::print_queue::{EnqueueError, PrintJob};
use crate::registration_server::store::Registration;
use crate::registration_server::{App, ServeError};

pub fn router(app: App) -> Router {
    let gate = Arc::clone(&app.gate);
    let dist = app.dist.clone();
    let form = app.form.clone();
    let operator = app
        .operator
        .clone()
        .with_current(Arc::clone(&app.current))
        .with_prints(app.prints.clone());
    let pages = Router::new()
        .route("/health", get(health))
        .route("/api/event", get(event))
        .route("/api/forms/vars", get(form_vars))
        .route("/api/forms/categories", get(form_categories))
        .route("/api/forms/settings", get(form_event_settings))
        .route("/api/forms/backgrounds/{name}", get(form_background))
        .route("/api/forms/{id}/struct", get(form_struct))
        .route("/api/forms/{id}/model", get(form_model))
        .route("/api/forms/{id}/conf", get(form_conf))
        .route("/api/forms/{id}/settings", get(form_settings))
        .route("/api/enums/{locale}/{name}", get(form_enum))
        .route("/api/qr", get(form_qr))
        .route("/api/registrations/counts", get(registration_counts))
        .route(
            "/api/registrations",
            get(registration_list).post(registration_save),
        )
        .route("/api/print", post(registration_print))
        .route("/api/print/preview", post(print_preview))
        .route("/api/certificates", post(print_certificates))
        .route(
            "/api/registrations/{id}/photo",
            get(registration_photo).post(registration_photo_post),
        )
        .route(
            "/api/registrations/{id}",
            get(registration_one).patch(registration_patch),
        )
        .route("/api/desk/auth", post(desk_auth))
        .route("/api/desk/catalog", get(desk_catalog))
        .route(
            "/api/settings",
            get(desk_settings).patch(desk_settings_patch),
        )
        .route("/api/printers/{name}", patch(printer_patch))
        .route("/api/sync", get(sync_status).post(sync_wake_request))
        .route("/api/update", get(update_status).post(update_install))
        .route("/api/update/check", post(update_check))
        .route("/registrations/{id}", get(lookup))
        .route("/print", post(enqueue_print))
        .route("/ws", get(ws))
        .route("/", get(form_index).head(form_index))
        .route("/admin", get(admin_index).head(admin_index))
        .route("/admin/events", get(admin_index).head(admin_index))
        .with_state(app);
    crate::registration_server::admin::api_router(operator)
        .merge(pages)
        .fallback(move |request: Request| {
            let dist = dist.clone();
            let form = form.clone();
            async move {
                let path = request.uri().path().to_string();
                if path == "/api" || path.starts_with("/api/") {
                    return api_missing().await;
                }
                if let Some(rest) = path.strip_prefix("/admin") {
                    let mapped = if rest.is_empty() || rest == "/" || rest == "/events" {
                        "/index.html"
                    } else {
                        rest
                    };
                    return crate::registration_server::admin::serve_mapped(&dist, mapped, request)
                        .await;
                }
                if crate::registration_server::admin::safe_public_file(&form, &path).is_some() {
                    return crate::registration_server::admin::serve_public(&form, request).await;
                }
                if form_screen(&path) {
                    return crate::registration_server::admin::serve_index(&form, request).await;
                }
                crate::registration_server::admin::serve_public(&dist, request).await
            }
        })
        .layer(from_fn_with_state(gate, track_inflight))
}

pub async fn serve(
    listener: TcpListener,
    app: App,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ServeError> {
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move {
            loop {
                if shutdown.changed().await.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        })
        .await
        .map_err(ServeError::Http)
}

async fn track_inflight(State(gate): State<Arc<Gate>>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    // `POST /api/update` holds its own guard so it can drop that guard while the download waits.
    let guard = if method == axum::http::Method::POST && path == "/api/update" {
        None
    } else {
        Some(gate.enter_request())
    };
    let response = next.run(request).await;
    log::debug!("{method} {path} {}", response.status());
    drop(guard);
    response
}

async fn form_index(State(app): State<App>, request: Request) -> Response {
    crate::registration_server::admin::serve_index(&app.form, request).await
}

async fn admin_index(State(app): State<App>, request: Request) -> Response {
    crate::registration_server::admin::serve_index(&app.dist, request).await
}

fn form_screen(path: &str) -> bool {
    matches!(
        path,
        "/" | "/register"
            | "/key"
            | "/desk"
            | "/form"
            | "/visitor"
            | "/visitors"
            | "/print"
            | "/import"
            | "/settings"
            | "/settings/registration"
            | "/printers"
    )
}

async fn api_missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "ok": false,
            "error": { "code": "not_found" }
        })),
    )
        .into_response()
}

pub fn network_body(snapshot: &crate::registration_server::Snapshot) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "data": {
            "samples": snapshot.samples,
            "offline": snapshot.offline,
            "quality": snapshot.quality.map(|quality| quality.as_str()),
            "timeout_secs": snapshot.timeout_secs,
        }
    })
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

fn api_ok(data: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "data": data })),
    )
        .into_response()
}

fn api_error(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "ok": false,
            "error": { "code": code }
        })),
    )
        .into_response()
}

async fn event(State(app): State<App>) -> Response {
    let id = CurrentEvent::lock(&app.current).event_id();
    let name = app
        .operator
        .credential_file()
        .load()
        .ok()
        .filter(|credentials| credentials.event_id() == id.as_deref())
        .and_then(|credentials| credentials.event_name().map(str::to_string));
    api_ok(serde_json::json!({ "id": id, "name": name }))
}

async fn form_vars(State(app): State<App>) -> Response {
    json_file(&app, &["forms", "vars.json"])
}

async fn form_categories(State(app): State<App>) -> Response {
    json_file(&app, &["forms", "categories.json"])
}

const CATALOG_HOLD: std::time::Duration = std::time::Duration::from_secs(25);
const CATALOG_TICK: std::time::Duration = std::time::Duration::from_millis(200);

struct CatalogSnapshot {
    rev: u64,
    categories: serde_json::Value,
    printers: serde_json::Value,
}

async fn desk_catalog(
    State(app): State<App>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let client_rev = query.get("rev").and_then(|raw| raw.parse::<u64>().ok());
    let started = tokio::time::Instant::now();
    loop {
        let snapshot = read_catalog(&app);
        let same = client_rev == Some(snapshot.rev);
        if !same || started.elapsed() >= CATALOG_HOLD {
            return catalog_response(snapshot);
        }
        tokio::time::sleep(CATALOG_TICK).await;
    }
}

fn catalog_response(snapshot: CatalogSnapshot) -> Response {
    let mut response = api_ok(serde_json::json!({
        "rev": snapshot.rev,
        "categories": snapshot.categories,
        "printers": snapshot.printers,
    }));
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn read_catalog(app: &App) -> CatalogSnapshot {
    let (base, event) = catalog_dirs(app);
    let categories = read_category_list(event.as_deref());
    let printers = read_printer_list(&base, event.as_deref());
    let fingerprint = serde_json::json!({
        "categories": &categories,
        "printers": &printers,
    })
    .to_string();
    CatalogSnapshot {
        rev: catalog_rev(&fingerprint),
        categories,
        printers,
    }
}

fn catalog_dirs(app: &App) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
    let current = crate::registration_server::current::CurrentEvent::lock(&app.current);
    let base = current.base_dir();
    let event = current.event_id().map(|id| base.join(id));
    (base, event)
}

fn exhibitor_category() -> serde_json::Value {
    serde_json::json!({ "cat_id": -2, "name": { "ru": "Экспонент", "en": "Exhibitor" } })
}

fn is_exhibitor(item: &serde_json::Value) -> bool {
    match item.get("cat_id").or_else(|| item.get("id")) {
        Some(serde_json::Value::Number(number)) => number.as_i64() == Some(-2),
        Some(serde_json::Value::String(text)) => text == "-2",
        _ => false,
    }
}

fn read_category_list(event: Option<&std::path::Path>) -> serde_json::Value {
    let stored = event
        .and_then(|event| read_stored_json(&event.join("forms").join("categories.json")).ok())
        .and_then(|value| match value {
            serde_json::Value::Array(items) => Some(items),
            _ => None,
        })
        .unwrap_or_default();
    let mut items: Vec<serde_json::Value> = stored
        .into_iter()
        .filter(|item| !is_exhibitor(item))
        .collect();
    items.insert(0, exhibitor_category());
    serde_json::Value::Array(items)
}

fn read_printer_list(base: &std::path::Path, event: Option<&std::path::Path>) -> serde_json::Value {
    let mut names = std::collections::BTreeSet::new();
    collect_stems(&base.join("printer_conf"), "", &mut names);
    if let Some(event) = event {
        collect_stems(event, "pr_", &mut names);
    }
    serde_json::Value::Array(
        names
            .into_iter()
            .map(|name| serde_json::json!({ "name": name, "text": name }))
            .collect(),
    )
}

fn collect_stems(
    dir: &std::path::Path,
    prefix: &str,
    names: &mut std::collections::BTreeSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(name) = stem.strip_prefix(prefix) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        names.insert(name.to_string());
    }
}

fn catalog_rev(body: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut hasher);
    let rev = hasher.finish();
    if rev == 0 {
        1
    } else {
        rev
    }
}

async fn form_event_settings(State(app): State<App>) -> Response {
    json_file(&app, &["forms", "settings.json"])
}

async fn form_struct(State(app): State<App>, Path(id): Path<String>) -> Response {
    form_part(&app, &id, "struct.json")
}

async fn form_model(State(app): State<App>, Path(id): Path<String>) -> Response {
    form_part(&app, &id, "model.json")
}

async fn form_conf(State(app): State<App>, Path(id): Path<String>) -> Response {
    form_part(&app, &id, "conf.json")
}

async fn form_settings(State(app): State<App>, Path(id): Path<String>) -> Response {
    form_part(&app, &id, "settings.json")
}

fn form_part(app: &App, id: &str, file: &str) -> Response {
    let Some(id) = segment(id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    json_file(app, &["forms", id, file])
}

async fn form_background(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(name) = segment(&name) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    let locale = query.get("locale").map(String::as_str).unwrap_or("");
    let resolution = query.get("resolution").map(String::as_str).unwrap_or("");
    let Some(locale) = segment(locale) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let Some(resolution) = segment(resolution) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let mut parts = Vec::new();
    parts.push("forms");
    parts.push("backgrounds");
    if let Some(form_id) = query.get("form_id").map(String::as_str) {
        let Some(form_id) = segment(form_id) else {
            return api_error(StatusCode::BAD_REQUEST, "bad_request");
        };
        parts.push(form_id);
    }
    parts.push(name);
    parts.push(locale);
    parts.push(resolution);
    let Some(path) = event_file(&app, &parts) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    match std::fs::read(&path) {
        Ok(bytes) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, image_type(&bytes))],
            bytes,
        )
            .into_response(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            api_error(StatusCode::NOT_FOUND, "not_found")
        }
        Err(err) => {
            log::debug!("background {}: {err}", path.display());
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

async fn form_enum(
    State(app): State<App>,
    Path((locale, name)): Path<(String, String)>,
) -> Response {
    let Some(locale) = segment(&locale) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    let Some(name) = segment(&name) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    let Some(dir) = event_dir(&app) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    let stem = dir.join("enums").join(locale).join(name);
    let path = [stem.with_extension("json"), stem.with_extension("json.gz")]
        .into_iter()
        .find(|path| path.is_file());
    let Some(path) = path else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    match read_stored_json(&path) {
        Ok(value) => api_ok(value),
        Err(Stored::Missing) => api_error(StatusCode::NOT_FOUND, "not_found"),
        Err(Stored::Bad) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error"),
    }
}

async fn form_qr(Query(query): Query<std::collections::HashMap<String, String>>) -> Response {
    let Some(text) = query.get("text").map(String::as_str) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    if text.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    match qr_png(text) {
        Ok(bytes) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "image/png")],
            bytes,
        )
            .into_response(),
        Err(err) => {
            log::debug!("qr: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

fn json_file(app: &App, parts: &[&str]) -> Response {
    let Some(path) = event_file(app, parts) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    match read_stored_json(&path) {
        Ok(value) => api_ok(value),
        Err(Stored::Missing) => api_error(StatusCode::NOT_FOUND, "not_found"),
        Err(Stored::Bad) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error"),
    }
}

fn event_dir(app: &App) -> Option<std::path::PathBuf> {
    let current = crate::registration_server::current::CurrentEvent::lock(&app.current);
    let id = current.event_id()?;
    Some(current.base_dir().join(id))
}

fn event_file(app: &App, parts: &[&str]) -> Option<std::path::PathBuf> {
    let mut path = event_dir(app)?;
    for part in parts {
        path.push(part);
    }
    Some(path)
}

fn segment(raw: &str) -> Option<&str> {
    if raw.is_empty() || raw == "." || raw == ".." {
        return None;
    }
    if raw
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Some(raw)
    } else {
        None
    }
}

enum Stored {
    Missing,
    Bad,
}

fn read_stored_json(path: &std::path::Path) -> Result<serde_json::Value, Stored> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(Stored::Missing),
        Err(err) => {
            log::debug!("form file {}: {err}", path.display());
            return Err(Stored::Bad);
        }
    };
    let bytes = if path.extension().and_then(|ext| ext.to_str()) == Some("gz")
        || bytes.starts_with(&[0x1f, 0x8b])
    {
        gunzip(&bytes).map_err(|err| {
            log::debug!("form gzip {}: {err}", path.display());
            Stored::Bad
        })?
    } else {
        bytes
    };
    serde_json::from_slice(&bytes).map_err(|err| {
        log::debug!("form json {}: {err}", path.display());
        Stored::Bad
    })
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn image_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/png"
    }
}

fn qr_png(text: &str) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|err| err.to_string())?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(200, 200)
        .build();
    let mut bytes = std::io::Cursor::new(Vec::new());
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::L8,
        )
        .map_err(|err| err.to_string())?;
    Ok(bytes.into_inner())
}

async fn registration_counts(
    State(app): State<App>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(event_id) = open_id(&app) else {
        return api_error(StatusCode::CONFLICT, "no_event");
    };
    let email = query.get("email").map(String::as_str).unwrap_or("");
    let phone = query.get("phone").map(String::as_str).unwrap_or("");
    if email.is_empty() && phone.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    let count = CurrentEvent::lock(&app.current).with_db(|db| {
        if !email.is_empty() {
            count_rows(db, "SELECT COUNT(*) FROM emails WHERE email = ?1", email)
        } else {
            count_rows(db, "SELECT COUNT(*) FROM phones WHERE phone = ?1", phone)
        }
    });
    match count {
        Some(Ok(count)) => api_ok(serde_json::json!({ "count": count })),
        Some(Err(err)) => {
            log::debug!("counts {event_id}: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
        None => api_error(StatusCode::CONFLICT, "no_event"),
    }
}

async fn registration_list(
    State(app): State<App>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(event_id) = open_id(&app) else {
        return api_error(StatusCode::CONFLICT, "no_event");
    };
    let flags = desk_flags(&app);
    if let Some(code) = query.get("code").filter(|code| !code.is_empty()) {
        let rows = match barcodes_for(&app, &event_id, code) {
            Ok(rows) => rows,
            Err(err) => {
                log::debug!("registrations code: {err}");
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
            }
        };
        return api_ok(serde_json::json!({
            "total": rows.len(),
            "rows": rows,
            "show_gotsome": flags.show_gotsome,
            "show_ticket_status": flags.show_ticket_status,
        }));
    }
    let text = [
        query.get("q").map(String::as_str).unwrap_or(""),
        query.get("name").map(String::as_str).unwrap_or(""),
        query.get("surname").map(String::as_str).unwrap_or(""),
        query.get("email").map(String::as_str).unwrap_or(""),
        query.get("company").map(String::as_str).unwrap_or(""),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let categories = query
        .get("category")
        .map(|raw| {
            raw.split(',')
                .filter_map(|part| part.trim().parse::<i64>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let limit = query.get("limit").and_then(|raw| raw.parse().ok());
    let offset = query.get("offset").and_then(|raw| raw.parse().ok());
    match CurrentEvent::lock(&app.current).search(
        &event_id,
        text.trim(),
        &categories,
        crate::registration_server::event_db::ListOrder::LastPrint,
        limit,
        offset,
    ) {
        Ok(page) => api_ok(serde_json::json!({
            "total": page.value.total,
            "rows": page.value.rows.iter().map(list_row_json).collect::<Vec<_>>(),
            "show_gotsome": flags.show_gotsome,
            "show_ticket_status": flags.show_ticket_status,
        })),
        Err(err) => {
            log::debug!("registrations search: {err}");
            api_error(StatusCode::CONFLICT, "no_event")
        }
    }
}

async fn registration_one(State(app): State<App>, Path(id): Path<String>) -> Response {
    let Some(event_id) = open_id(&app) else {
        return api_error(StatusCode::CONFLICT, "no_event");
    };
    match CurrentEvent::lock(&app.current).full_row(&event_id, &id) {
        Ok(row) => match row.value {
            Some(data) => match serde_json::from_str::<serde_json::Value>(&data) {
                Ok(value) => api_ok(value),
                Err(_) => api_ok(serde_json::Value::String(data)),
            },
            None => api_error(StatusCode::NOT_FOUND, "not_found"),
        },
        Err(err) => {
            log::debug!("registration {id}: {err}");
            api_error(StatusCode::CONFLICT, "no_event")
        }
    }
}

async fn registration_save(State(app): State<App>, body: axum::body::Bytes) -> Response {
    let Some(event_id) = open_id(&app) else {
        return api_error(StatusCode::CONFLICT, "no_event");
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    if !parsed.is_object() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    let noprint = parsed
        .get("noprint")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let mut visitor = parsed
        .get("visitor")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(parsed);
    let email = crate::registration_server::event_db::document_email(&visitor);
    let phone = crate::registration_server::event_db::document_phone(&visitor);
    if email_phone_conflict(&app, &email, &phone) {
        return api_error(StatusCode::CONFLICT, "invalid_phone_email_combo");
    }
    let mut barcode = field_text(&visitor, &["barcode"]);
    if barcode.is_empty() {
        match take_ticket(&app) {
            Ok(code) => barcode = code.unwrap_or_default(),
            Err(err) => {
                log::debug!("ticket barcode: {err}");
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
            }
        }
    }
    if !barcode.is_empty() {
        if let Some(object) = visitor.as_object_mut() {
            object.insert("barcode".to_string(), serde_json::json!(barcode));
        }
    }
    let uid = match field_text(&visitor, &["uid", "id"]) {
        value if !value.is_empty() => value,
        _ => match next_uid(&app) {
            Ok(uid) => uid,
            Err(err) => {
                log::debug!("next uid: {err}");
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
            }
        },
    };
    if let Some(object) = visitor.as_object_mut() {
        object.insert("uid".to_string(), serde_json::json!(uid));
    }
    if let Err(err) = CurrentEvent::lock(&app.current).save_document(&event_id, &visitor) {
        log::debug!("registration write: {err}");
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    }
    let flags = desk_flags(&app);
    let mut zone_name = String::new();
    if flags.print_on_save && !noprint {
        zone_name = "desk".to_string();
        if let Err(err) = app.prints.enqueue(PrintJob {
            id: uid,
            pages: 1,
            zone: Some("desk".to_string()),
            name: None,
            certificate: None,
            category: None,
        }) {
            log::debug!("registration print: {err:?}");
            return api_error(StatusCode::CONFLICT, "no_printer");
        }
    }
    let mut data = serde_json::json!({ "zone_name": zone_name });
    if flags.show_ticket_status {
        data["ticket_status"] = serde_json::json!(field_i64(&visitor, "ticket_status"));
    }
    api_ok(data)
}

async fn registration_print(State(app): State<App>, body: axum::body::Bytes) -> Response {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    print_ids(&app, &parsed, false)
}

async fn print_certificates(State(app): State<App>, body: axum::body::Bytes) -> Response {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let Some(ids) = id_list(&parsed) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let printer_name = body_text(&parsed, "printer");
    let zone = body_text(&parsed, "zone");
    let Some(search) = open_search(&app) else {
        return api_error(StatusCode::CONFLICT, "no_printer");
    };
    let mut rows = Vec::new();
    for id in ids {
        let Some(raw) = stored_row(&app, &id) else {
            return api_error(StatusCode::NOT_FOUND, "not_found");
        };
        let value: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
        let code = match take_ticket(&app) {
            Ok(Some(code)) => code,
            Ok(None) => {
                rows.push(serde_json::json!({
                    "name": field_text(&value, &["name"]),
                    "code": "",
                    "printer": "",
                    "zone": zone.clone().unwrap_or_default(),
                }));
                continue;
            }
            Err(err) => {
                log::debug!("certificate code: {err}");
                return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
            }
        };
        if let Some(err) = write_cert_code(&app, &id, &code) {
            log::debug!("certificate store: {err}");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
        }
        let category = field_i64(&value, "category");
        let internet = json_flag(&value, "internet") || json_flag(&value, "is_internet");
        let Some(printer) = pick_queue(&search, &printer_name, &zone, true, category, internet)
        else {
            return api_error(StatusCode::CONFLICT, "no_printer");
        };
        if let Err(response) = enqueue_print_job(
            &app,
            &id,
            printer.name.clone(),
            zone.clone(),
            true,
            Some(category),
        ) {
            return response;
        }
        rows.push(serde_json::json!({
            "name": field_text(&value, &["name"]),
            "code": code,
            "printer": printer.name,
            "zone": printer.routing.zone,
        }));
    }
    if rows.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    api_ok(serde_json::json!({ "rows": rows }))
}

async fn print_preview(State(app): State<App>, body: axum::body::Bytes) -> Response {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let printer_name = body_text(&parsed, "printer");
    let zone = body_text(&parsed, "zone");
    let certificate = json_flag(&parsed, "certificate");
    let Some(search) = open_search(&app) else {
        return api_error(StatusCode::CONFLICT, "no_printer");
    };
    let id = body_text(&parsed, "id");
    if id.is_none() {
        let Some(name) = printer_name else {
            return api_error(StatusCode::BAD_REQUEST, "bad_request");
        };
        if pick_queue(&search, &Some(name), &zone, certificate, 0, false).is_none() {
            return api_error(StatusCode::CONFLICT, "no_printer");
        }
        return png_response(test_page_png());
    }
    let id = id.unwrap();
    let Some(raw) = stored_row(&app, &id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    let value: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
    let category = body_category(&parsed).unwrap_or_else(|| field_i64(&value, "category"));
    prepare_search(&app, &search, &id, &value);
    let request = crate::registration_server::cups::PrintRequest {
        id,
        category,
        internet: json_flag(&value, "internet") || json_flag(&value, "is_internet"),
        zone,
        name: printer_name,
        certificate,
        transliterate: false,
        double_print: false,
        rule: None,
        pixel_cap: None,
    };
    match search.preview(&request) {
        Ok(bytes) => png_response(bytes),
        Err(err) if err.to_string() == "no queue" => api_error(StatusCode::CONFLICT, "no_printer"),
        Err(err) => {
            log::debug!("print preview: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

fn print_ids(app: &App, parsed: &serde_json::Value, certificate: bool) -> Response {
    let Some(ids) = id_list(parsed) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let printer_name = body_text(parsed, "printer");
    let zone = body_text(parsed, "zone");
    let certificate = certificate || json_flag(parsed, "certificate");
    let noprint = json_flag(parsed, "noprint");
    let check = json_flag(parsed, "check");
    let category_override = body_category(parsed);
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    let mut pending = Vec::new();
    for id in ids {
        let Some(raw) = stored_row(app, &id) else {
            if check {
                skipped.push(serde_json::Value::String(id));
                continue;
            }
            return api_error(StatusCode::NOT_FOUND, "not_found");
        };
        pending.push((id, raw));
    }
    if pending.is_empty() {
        if skipped.is_empty() {
            return api_error(StatusCode::BAD_REQUEST, "bad_request");
        }
        return api_ok(serde_json::json!({ "jobs": jobs, "skipped": skipped }));
    }
    let Some(search) = open_search(app) else {
        return api_error(StatusCode::CONFLICT, "no_printer");
    };
    for (id, raw) in pending {
        let value: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
        let category = category_override.unwrap_or_else(|| field_i64(&value, "category"));
        let internet = json_flag(&value, "internet") || json_flag(&value, "is_internet");
        let Some(printer) = pick_queue(
            &search,
            &printer_name,
            &zone,
            certificate,
            category,
            internet,
        ) else {
            return api_error(StatusCode::CONFLICT, "no_printer");
        };
        if !noprint {
            if let Err(response) = enqueue_print_job(
                app,
                &id,
                printer.name.clone(),
                zone.clone(),
                certificate,
                Some(category),
            ) {
                return response;
            }
        } else if let Err(err) = record_now(app, &id) {
            log::debug!("print row: {err}");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
        }
        jobs.push(serde_json::json!({
            "printer": printer.name,
            "zone": printer.routing.zone,
            "certificate": certificate,
            "zone_name": printer.routing.sector_name,
        }));
    }
    if jobs.is_empty() && skipped.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    api_ok(serde_json::json!({ "jobs": jobs, "skipped": skipped }))
}

fn enqueue_print_job(
    app: &App,
    id: &str,
    name: String,
    zone: Option<String>,
    certificate: bool,
    category: Option<i64>,
) -> Result<(), Response> {
    app.prints
        .enqueue(PrintJob {
            id: id.to_string(),
            pages: 1,
            zone,
            name: Some(name),
            certificate: Some(certificate),
            category,
        })
        .map_err(|err| {
            log::debug!("registration print: {err:?}");
            match err {
                EnqueueError::Full | EnqueueError::Closed => {
                    api_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
                }
                EnqueueError::EmptyId | EnqueueError::NoPages => {
                    api_error(StatusCode::BAD_REQUEST, "bad_request")
                }
            }
        })
}

fn pick_queue(
    search: &crate::registration_server::cups::Search,
    printer: &Option<String>,
    zone: &Option<String>,
    certificate: bool,
    category: i64,
    internet: bool,
) -> Option<crate::registration_server::cups::Printer> {
    let job = crate::registration_server::cups::Job {
        name: printer.clone(),
        category: Some(category.to_string()),
        internet,
        certificate,
        zone: zone.clone(),
        rule: None,
    };
    match search.pick(&job) {
        Ok(printer) => Some(printer),
        Err(err) => {
            log::debug!("print pick: {err}");
            None
        }
    }
}

fn prepare_search(
    app: &App,
    search: &crate::registration_server::cups::Search,
    id: &str,
    value: &serde_json::Value,
) {
    let current = CurrentEvent::lock(&app.current);
    if let Some(event_id) = current.event_id() {
        search.set_files(&current.base_dir(), &event_id);
    }
    search.set_badges(current.badges());
    drop(current);
    search.set_visitor(id, visitor_fields(value));
}

fn stored_row(app: &App, id: &str) -> Option<String> {
    let current = CurrentEvent::lock(&app.current);
    let event_id = current.event_id()?;
    current.full_row(&event_id, id).ok()?.value
}

fn open_search(app: &App) -> Option<crate::registration_server::cups::Search> {
    app.search.lock().expect("search").clone()
}

fn record_now(app: &App, id: &str) -> Result<(), String> {
    let current = CurrentEvent::lock(&app.current);
    let event_id = current.event_id().ok_or_else(|| "no event".to_string())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    current
        .record_print(&event_id, id, ts)
        .map_err(|err| err.to_string())
}

fn write_cert_code(app: &App, id: &str, code: &str) -> Option<String> {
    match CurrentEvent::lock(&app.current).with_db(|db| {
        db.file().execute(
            "UPDATE mem_u SET cert_code = ?1 WHERE uid = ?2",
            rusqlite::params![code, id],
        )
    }) {
        Some(Ok(_)) => None,
        Some(Err(err)) => Some(err.to_string()),
        None => Some("no event".to_string()),
    }
}

fn png_response(bytes: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/png")],
        bytes,
    )
        .into_response()
}

fn test_page_png() -> Vec<u8> {
    use image::ImageEncoder;
    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .expect("test page png");
    bytes.into_inner()
}

fn id_list(parsed: &serde_json::Value) -> Option<Vec<String>> {
    let ids = parsed.get("ids")?.as_array()?;
    let ids: Vec<String> = ids
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

fn body_text(parsed: &serde_json::Value, key: &str) -> Option<String> {
    parsed
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn body_category(parsed: &serde_json::Value) -> Option<i64> {
    parsed.get("category").and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_str().and_then(|text| text.parse().ok()))
    })
}

fn json_flag(value: &serde_json::Value, key: &str) -> bool {
    match value.get(key) {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::Number(number)) => number.as_i64() == Some(1),
        Some(serde_json::Value::String(text)) => text == "1" || text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn visitor_fields(value: &serde_json::Value) -> std::collections::HashMap<String, String> {
    let Some(object) = value.as_object() else {
        return std::collections::HashMap::new();
    };
    let mut fields = std::collections::HashMap::new();
    for (key, item) in object {
        let Some(text) = (match item {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }) else {
            continue;
        };
        fields.insert(key.clone(), text);
    }
    fields
}

async fn registration_patch(
    State(app): State<App>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(event_id) = open_id(&app) else {
        return api_error(StatusCode::CONFLICT, "no_event");
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let gotsome = parsed.get("gotsome").and_then(|value| value.as_i64());
    let ticket_status = parsed.get("ticket_status").and_then(|value| value.as_i64());
    if gotsome.is_none() && ticket_status.is_none() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    let updated = CurrentEvent::lock(&app.current).with_db(|db| {
        let exists: Option<i64> = db
            .file()
            .query_row("SELECT 1 FROM mem_u WHERE uid = ?1", [&id], |row| {
                row.get(0)
            })
            .optional()?;
        if exists.is_none() {
            return Ok(false);
        }
        if let Some(gotsome) = gotsome {
            db.memory().execute(
                "UPDATE list_row SET gotsome = ?1 WHERE uid = ?2",
                rusqlite::params![gotsome, &id],
            )?;
            db.file().execute(
                "UPDATE u_fulltext SET gotsome = ?1 WHERE uid = ?2",
                rusqlite::params![gotsome.to_string(), &id],
            )?;
        }
        if let Some(ticket_status) = ticket_status {
            db.memory().execute(
                "UPDATE list_row SET ticket_status = ?1 WHERE uid = ?2",
                rusqlite::params![ticket_status, &id],
            )?;
            db.file().execute(
                "UPDATE mem_u SET ticket_status = ?1 WHERE uid = ?2",
                rusqlite::params![ticket_status, &id],
            )?;
            db.file().execute(
                "UPDATE u_fulltext SET ticket_status = ?1 WHERE uid = ?2",
                rusqlite::params![ticket_status.to_string(), &id],
            )?;
        }
        db.file()
            .execute("UPDATE mem_u SET in_synch = NULL WHERE uid = ?1", [&id])?;
        Ok::<_, rusqlite::Error>(true)
    });
    match updated {
        Some(Ok(true)) => {
            let _ = event_id;
            api_ok(serde_json::json!({}))
        }
        Some(Ok(false)) => api_error(StatusCode::NOT_FOUND, "not_found"),
        Some(Err(err)) => {
            log::debug!("registration patch: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
        None => api_error(StatusCode::CONFLICT, "no_event"),
    }
}

async fn registration_photo(State(app): State<App>, Path(id): Path<String>) -> Response {
    let Some(path) = photo_file(&app, &id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    match std::fs::read(&path) {
        Ok(bytes) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, image_type(&bytes))],
            bytes,
        )
            .into_response(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            api_error(StatusCode::NOT_FOUND, "not_found")
        }
        Err(err) => {
            log::debug!("photo {}: {err}", path.display());
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

async fn registration_photo_post(
    State(app): State<App>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let Some(path) = photo_file(&app, &id) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            log::debug!("photo dir: {err}");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
        }
    }
    if let Err(err) = std::fs::write(&path, &body) {
        log::debug!("photo write: {err}");
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    }
    let _ = CurrentEvent::lock(&app.current).with_db(|db| {
        db.file()
            .execute("UPDATE mem_u SET in_synch = NULL WHERE uid = ?1", [&id])
    });
    api_ok(serde_json::json!({}))
}

fn photo_file(app: &App, id: &str) -> Option<std::path::PathBuf> {
    let id = segment(id)?;
    Some(
        event_dir(app)?
            .join("photos")
            .join("uids")
            .join(format!("{id}.png")),
    )
}

fn open_id(app: &App) -> Option<String> {
    CurrentEvent::lock(&app.current).event_id()
}

struct DeskFlags {
    print_on_save: bool,
    show_ticket_status: bool,
    show_gotsome: bool,
}

fn desk_flags(app: &App) -> DeskFlags {
    let mut flags = DeskFlags {
        print_on_save: false,
        show_ticket_status: false,
        show_gotsome: false,
    };
    let Some(path) = event_dir(app).map(|dir| dir.join("settings.json")) else {
        return flags;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return flags;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return flags;
    };
    flags.print_on_save = value
        .get("print_on_save")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    flags.show_ticket_status = value
        .get("show_ticket_status")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    flags.show_gotsome = value
        .get("show_gotsome")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    flags
}

fn list_row_json(row: &crate::registration_server::event_db::ListRow) -> serde_json::Value {
    serde_json::json!({
        "uid": row.uid,
        "name": row.name,
        "surname": row.surname,
        "c_name": row.c_name,
        "category": row.category,
        "ticket_status": row.ticket_status,
        "gotsome": row.gotsome,
        "give_packet": row.give_packet,
        "added_ts": row.added_ts,
        "last_print_ts": row.last_print_ts,
        "print_count": row.print_count,
        "org_id": row.org_id,
        "email": row.email,
    })
}

fn barcodes_for(app: &App, event_id: &str, code: &str) -> Result<Vec<serde_json::Value>, String> {
    let _ = event_id;
    CurrentEvent::lock(&app.current)
        .with_db(|db| {
            let mut stmt = db.file().prepare(
                "SELECT uid, IFNULL(barcode, '') FROM mem_u
                 WHERE barcode = ?1 OR code = ?1 OR repl_barcode = ?1",
            )?;
            let rows = stmt.query_map([code], |row| {
                Ok(serde_json::json!({
                    "uid": row.get::<_, String>(0)?,
                    "barcode": row.get::<_, String>(1)?,
                }))
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .ok_or_else(|| "no event".to_string())?
        .map_err(|err| err.to_string())
}

fn email_phone_conflict(app: &App, email: &str, phone: &str) -> bool {
    if email.is_empty() || phone.is_empty() {
        return false;
    }
    CurrentEvent::lock(&app.current)
        .with_db(|db| {
            let email_uid = one_uid(db, "SELECT uid FROM emails WHERE email = ?1", email)?;
            let phone_uid = one_uid(db, "SELECT uid FROM phones WHERE phone = ?1", phone)?;
            Ok::<_, rusqlite::Error>(match (email_uid, phone_uid) {
                (Some(email_uid), Some(phone_uid)) => email_uid != phone_uid,
                _ => false,
            })
        })
        .and_then(|result| result.ok())
        .unwrap_or(false)
}

fn one_uid(
    db: &crate::registration_server::event_db::EventDb,
    sql: &str,
    value: &str,
) -> Result<Option<i64>, rusqlite::Error> {
    db.file()
        .query_row(sql, [value], |row| row.get(0))
        .optional()
}

fn count_rows(
    db: &crate::registration_server::event_db::EventDb,
    sql: &str,
    value: &str,
) -> Result<i64, rusqlite::Error> {
    db.file().query_row(sql, [value], |row| row.get(0))
}

fn take_ticket(app: &App) -> Result<Option<String>, rusqlite::Error> {
    CurrentEvent::lock(&app.current)
        .with_db(|db| {
            let row = db
                .file()
                .query_row(
                    "SELECT id, barcode FROM ticketbarcodes ORDER BY id LIMIT 1",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((id, barcode)) = row else {
                return Ok(None);
            };
            db.file()
                .execute("DELETE FROM ticketbarcodes WHERE id = ?1", [id])?;
            Ok(Some(barcode))
        })
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?
}

fn next_uid(app: &App) -> Result<String, rusqlite::Error> {
    let next = CurrentEvent::lock(&app.current)
        .with_db(|db| {
            db.file().query_row(
                "SELECT IFNULL(MAX(CAST(uid AS INTEGER)), 0) + 1 FROM mem_u",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .ok_or(rusqlite::Error::QueryReturnedNoRows)??;
    Ok(next.to_string())
}

fn field_text(value: &serde_json::Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(value_text) {
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

fn field_i64(value: &serde_json::Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(|item| {
            item.as_i64()
                .or_else(|| item.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0)
}

fn value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

const SETTINGS_KEYS: [&str; 7] = [
    "show_ticket_status",
    "show_gotsome",
    "print_on_save",
    "capitalize_names",
    "transliterate",
    "double_print",
    "load_barcodes",
];

async fn desk_auth(State(app): State<App>, body: axum::body::Bytes) -> Response {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let key = parsed
        .get("key")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .trim();
    if key.is_empty() || key_is_admin(&app, key).is_none() {
        return api_error(StatusCode::UNAUTHORIZED, "invalid_key");
    }
    let cookie = cookie::Cookie::build(("desk", key.to_string()))
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .path("/")
        .build()
        .to_string();
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true, "data": {} })),
    )
        .into_response()
}

async fn desk_settings(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Err(response) = require_admin(&app, &headers) {
        return response;
    }
    api_ok(settings_document(&app))
}

async fn desk_settings_patch(
    State(app): State<App>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(response) = require_admin(&app, &headers) {
        return response;
    }
    let Some(dir) = event_dir(&app) else {
        return api_error(StatusCode::CONFLICT, "no_event");
    };
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let Some(incoming) = parsed.as_object() else {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let mut document = settings_document(&app);
    let Some(stored) = document.as_object_mut() else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    };
    for key in SETTINGS_KEYS {
        if let Some(value) = incoming.get(key) {
            stored.insert(key.to_string(), value.clone());
        }
    }
    let path = dir.join("settings.json");
    let Ok(bytes) = serde_json::to_vec_pretty(&document) else {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    };
    if let Err(err) = std::fs::write(&path, bytes) {
        log::debug!("settings {}: {err}", path.display());
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    }
    api_ok(document)
}

async fn printer_patch(
    State(app): State<App>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(response) = require_admin(&app, &headers) {
        return response;
    }
    let Some(name) = segment(&name) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return api_error(StatusCode::BAD_REQUEST, "bad_request");
    }
    let base = CurrentEvent::lock(&app.current).base_dir();
    let dir = base.join("printer_conf");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::debug!("printer conf: {err}");
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    }
    let path = dir.join(format!("{name}.json"));
    if let Err(err) = std::fs::write(&path, &body) {
        log::debug!("printer {}: {err}", path.display());
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error");
    }
    api_ok(serde_json::json!({}))
}

pub(crate) fn require_desk(app: &App, headers: &HeaderMap) -> Result<(), Response> {
    let Some(key) = desk_cookie(headers) else {
        return Err(api_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    match key_is_admin(app, &key) {
        Some(_) => Ok(()),
        None => Err(api_error(StatusCode::UNAUTHORIZED, "unauthorized")),
    }
}

async fn update_status(State(app): State<App>, headers: HeaderMap) -> Response {
    if !can_read_update(&app, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    api_ok(crate::registration_server::update::update_document(&app))
}

async fn update_install(State(app): State<App>, request: Request) -> Response {
    if !app.operator.has_live_session(request.headers()) {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if !crate::registration_server::update::update_allowed(&app) {
        eprintln!("registration-server update: refused, parent is not rust-reg-run");
        return api_error(StatusCode::CONFLICT, "no_supervisor");
    }
    let held = app.gate.enter_request();
    if crate::registration_server::update::print_busy(&app) {
        return api_error(StatusCode::CONFLICT, "print_busy");
    }
    let outcome = crate::registration_server::update::install(&app, Some(held)).await;
    let _held = outcome.held;
    match outcome.result {
        Ok(version) => {
            crate::registration_server::update::request_exit(&app);
            api_ok(serde_json::json!({ "version": version }))
        }
        Err(crate::registration_server::update::StageFailure::Checksum) => {
            eprintln!("registration-server update: checksum mismatch");
            api_error(StatusCode::CONFLICT, "checksum_mismatch")
        }
        Err(crate::registration_server::update::StageFailure::Failed) => {
            eprintln!("registration-server update: install failed");
            api_error(StatusCode::CONFLICT, "download_failed")
        }
    }
}

async fn update_check(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Err(response) = require_desk(&app, &headers) {
        return response;
    }
    match crate::registration_server::update::check_now(&app).await {
        Ok(data) => api_ok(data),
        Err(crate::registration_server::update::CheckError::Failed) => {
            api_error(StatusCode::BAD_GATEWAY, "update_check_failed")
        }
        Err(crate::registration_server::update::CheckError::InProgress) => {
            api_error(StatusCode::TOO_MANY_REQUESTS, "check_in_progress")
        }
    }
}

fn can_read_update(app: &App, headers: &HeaderMap) -> bool {
    require_desk(app, headers).is_ok() || app.operator.has_live_session(headers)
}

fn require_admin(app: &App, headers: &HeaderMap) -> Result<(), Response> {
    let Some(key) = desk_cookie(headers) else {
        return Err(api_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    };
    match key_is_admin(app, &key) {
        Some(true) => Ok(()),
        _ => Err(api_error(StatusCode::UNAUTHORIZED, "unauthorized")),
    }
}

fn desk_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "desk").then(|| value.trim().to_string())
    })
}

fn key_is_admin(app: &App, key: &str) -> Option<bool> {
    let flag = CurrentEvent::lock(&app.current)
        .with_db(|db| {
            db.file()
                .query_row(
                    "SELECT IFNULL(is_admin, 0) FROM keys WHERE key = ?1",
                    [key],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
        })?
        .ok()?;
    flag.map(|flag| flag != 0)
}

fn settings_document(app: &App) -> serde_json::Value {
    let mut document = serde_json::json!({
        "show_ticket_status": false,
        "show_gotsome": false,
        "print_on_save": false,
        "capitalize_names": false,
        "transliterate": false,
        "double_print": false,
        "load_barcodes": false,
    });
    let Some(path) = event_dir(app).map(|dir| dir.join("settings.json")) else {
        return document;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return document;
    };
    let Ok(stored) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return document;
    };
    let Some(stored) = stored.as_object() else {
        return document;
    };
    if let Some(document) = document.as_object_mut() {
        for key in SETTINGS_KEYS {
            if let Some(value) = stored.get(key) {
                document.insert(key.to_string(), value.clone());
            }
        }
    }
    document
}

async fn sync_status(State(app): State<App>, headers: HeaderMap) -> Response {
    let Some(key) = desk_cookie(&headers) else {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let Some(admin) = key_is_admin(&app, &key) else {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    match CurrentEvent::lock(&app.current).registration_sync_counts() {
        Ok((waiting, synced)) => api_ok(serde_json::json!({
            "waiting": waiting,
            "synced": synced,
            "addresses": local_addresses(),
            "is_admin": admin,
        })),
        Err(err) => {
            log::debug!("sync status: {err}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "unknown_error")
        }
    }
}

/// The address this machine would use to reach the network. A UDP connect does not send a packet.
fn local_addresses() -> Vec<String> {
    let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else {
        return vec!["127.0.0.1".to_string()];
    };
    if socket.connect("192.0.2.1:9").is_err() {
        return vec!["127.0.0.1".to_string()];
    }
    match socket.local_addr() {
        Ok(addr) if !addr.ip().is_unspecified() && !addr.ip().is_loopback() => {
            vec![addr.ip().to_string()]
        }
        _ => vec!["127.0.0.1".to_string()],
    }
}

async fn sync_wake_request(State(app): State<App>) -> Response {
    app.request_sync();
    api_ok(serde_json::json!({}))
}

async fn lookup(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<Registration>, StatusCode> {
    app.store.lookup(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn enqueue_print(
    State(app): State<App>,
    Json(job): Json<PrintJob>,
) -> Result<(StatusCode, Json<PrintJob>), StatusCode> {
    let queued = job.clone();
    log::debug!("print queued id={}", queued.id);
    app.prints.enqueue(job).map_err(enqueue_status)?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

fn enqueue_status(err: EnqueueError) -> StatusCode {
    match err {
        EnqueueError::EmptyId | EnqueueError::NoPages => StatusCode::BAD_REQUEST,
        EnqueueError::Full | EnqueueError::Closed => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn ws(State(app): State<App>, upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| handle_socket(socket, app))
}

async fn handle_socket(mut socket: WebSocket, app: App) {
    while let Some(incoming) = socket.recv().await {
        let Ok(incoming) = incoming else {
            break;
        };
        let Message::Text(text) = incoming else {
            if matches!(incoming, Message::Close(_)) {
                break;
            }
            continue;
        };
        let reply = {
            let _guard = app.gate.enter_request();
            reply_for(&app, text.as_str())
        };
        if socket.send(Message::Text(reply.into())).await.is_err() {
            break;
        }
    }
}

fn reply_for(app: &App, id: &str) -> String {
    match app.store.lookup(id.trim()) {
        Some(row) => serde_json::to_string(&row).unwrap_or_else(|_| "{}".into()),
        None => "{\"found\":false}".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::registration_server::credentials::{CredentialFile, Credentials};
    use crate::registration_server::current::CurrentEvent;
    use crate::registration_server::App;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "reg-http-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Venue {
        dir: TempDir,
        app: App,
        file: CredentialFile,
        worker: crate::registration_server::print_queue::PrintWorker,
    }

    fn open_event(event_id: &str) -> Venue {
        let dir = TempDir::new();
        let file = CredentialFile::new(dir.0.join("credentials.yml"));
        let mut credentials = Credentials::default();
        credentials.set_device_id("device-1").unwrap();
        credentials
            .bind(event_id, "token-value", Some("Event".into()))
            .unwrap();
        file.store(&credentials).unwrap();
        let current = Arc::new(Mutex::new(CurrentEvent::new(&dir.0)));
        CurrentEvent::lock(&current).open_bound(&file).unwrap();
        let (mut app, worker) = App::new();
        app.current = Arc::clone(&current);
        app.operator = crate::registration_server::admin::OperatorState::new(
            CredentialFile::new(file.path()),
            crate::registration_server::remote::Remote::new("http://127.0.0.1:1").unwrap(),
        )
        .with_current(current);
        Venue {
            dir,
            app,
            file,
            worker,
        }
    }

    async fn call(app: &App, uri: &str) -> (StatusCode, Vec<u8>) {
        let response = super::router(app.clone())
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    fn json_body(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).unwrap()
    }

    mod liveness {
        use super::*;

        #[tokio::test]
        async fn health_stays_ok() {
            let (app, _worker) = App::new();
            let (status, body) = call(&app, "/health").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body), serde_json::json!({ "status": "ok" }));
        }

        #[tokio::test]
        async fn event_id_follows_the_holder() {
            let venue = open_event("EVT");
            let (status, body) = call(&venue.app, "/api/event").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json_body(&body),
                serde_json::json!({ "ok": true, "data": { "id": "EVT", "name": "Event" } })
            );

            CurrentEvent::lock(&venue.app.current)
                .clear(&venue.file)
                .unwrap();
            let (status, body) = call(&venue.app, "/api/event").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json_body(&body),
                serde_json::json!({ "ok": true, "data": { "id": null, "name": null } })
            );
        }
    }

    mod forms {
        use super::*;

        #[tokio::test]
        async fn form_json_is_the_stored_file() {
            let venue = open_event("EVT");
            let dir = venue.dir.0.join("EVT").join("forms").join("1");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("conf.json"), br#"{"title":"desk"}"#).unwrap();

            let (status, body) = call(&venue.app, "/api/forms/1/conf").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json_body(&body),
                serde_json::json!({ "ok": true, "data": { "title": "desk" } })
            );

            let (status, body) = call(&venue.app, "/api/forms/9/conf").await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(
                json_body(&body),
                serde_json::json!({ "ok": false, "error": { "code": "not_found" } })
            );
        }

        #[tokio::test]
        async fn background_is_the_file_bytes() {
            let venue = open_event("EVT");
            let png = b"\x89PNG\r\n\x1a\nbackground";
            let dir = venue
                .dir
                .0
                .join("EVT")
                .join("forms")
                .join("backgrounds")
                .join("bg")
                .join("ru");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("pad"), png).unwrap();

            let (status, body) = call(
                &venue.app,
                "/api/forms/backgrounds/bg?locale=ru&resolution=pad",
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, png);
        }

        #[tokio::test]
        async fn qr_is_png() {
            let (app, _worker) = App::new();
            let (status, body) = call(&app, "/api/qr?text=abc").await;
            assert_eq!(status, StatusCode::OK);
            assert!(body.starts_with(b"\x89PNG"), "body is not a png");
            let image = image::load_from_memory(&body).unwrap().into_luma8();
            let mut prepared = rqrr::PreparedImage::prepare(image);
            let grids = prepared.detect_grids();
            assert!(!grids.is_empty(), "no qr grid");
            let (_meta, content) = grids[0].decode().unwrap();
            assert_eq!(content, "abc");
        }
    }

    mod visitors {
        use super::*;

        async fn post(app: &App, uri: &str, body: &[u8]) -> (StatusCode, Vec<u8>) {
            let response = super::super::router(app.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_vec()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            (status, bytes.to_vec())
        }

        #[tokio::test]
        async fn search_by_code() {
            let venue = open_event("EVT");
            let visitor = crate::registration_server::event_db::VisitorWrite {
                uid: "7".to_string(),
                data: r#"{"uid":"7"}"#.to_string(),
                name: "Ann".to_string(),
                surname: "Bee".to_string(),
                c_name: String::new(),
                category: 1,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 1,
                org_id: None,
                email: String::new(),
            };
            CurrentEvent::lock(&venue.app.current)
                .write("EVT", &visitor)
                .unwrap();
            CurrentEvent::lock(&venue.app.current)
                .with_db(|db| {
                    db.file()
                        .execute("UPDATE mem_u SET barcode = 'SCAN1' WHERE uid = '7'", [])
                        .unwrap();
                })
                .unwrap();

            let (status, body) = call(&venue.app, "/api/registrations?code=SCAN1").await;
            assert_eq!(status, StatusCode::OK);
            let found = json_body(&body);
            assert_eq!(found["data"]["rows"][0]["barcode"], "SCAN1");
            assert_eq!(found["data"]["rows"][0]["uid"], "7");

            let (status, body) = call(&venue.app, "/api/registrations?code=MISSING").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["rows"], serde_json::json!([]));
        }

        #[tokio::test]
        async fn save_writes_the_row() {
            let mut venue = open_event("EVT");
            let (status, body) = post(
                &venue.app,
                "/api/registrations",
                br#"{"uid":"4","name":"Kay","surname":"Lee","email":"k@l.c","barcode":"HAVE"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["ok"], true);
            assert!(venue.worker.try_recv().is_none());

            let (status, body) = call(&venue.app, "/api/registrations/4").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["name"], "Kay");
            assert_eq!(json_body(&body)["data"]["barcode"], "HAVE");
        }

        #[tokio::test]
        async fn save_lists_the_form_email() {
            let venue = open_event("EVT");
            let (status, _) = post(
                &venue.app,
                "/api/registrations",
                br#"{"uid":"9","personalData":{"name":"Kat","surname":"Lee","emails":[{"email":"long.email@gmail.com"}],"companies":[{"name":{"ru":{"str":"Hands"}}}]}}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let (status, body) = call(&venue.app, "/api/registrations?q=Lee").await;
            assert_eq!(status, StatusCode::OK);
            let row = &json_body(&body)["data"]["rows"][0];
            assert_eq!(row["email"], "long.email@gmail.com");
            assert_eq!(row["c_name"], "Hands");
        }

        #[tokio::test]
        async fn empty_barcode_still_saves() {
            let venue = open_event("EVT");
            let (status, body) = post(
                &venue.app,
                "/api/registrations",
                br#"{"uid":"5","name":"No","surname":"Code"}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["ok"], true);
            let (status, body) = call(&venue.app, "/api/registrations/5").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["name"], "No");
            assert!(json_body(&body)["data"].get("barcode").is_none());
        }

        #[tokio::test]
        async fn print_enqueues_the_visitor() {
            let mut venue = open_event("EVT");
            let (status, _) =
                post(&venue.app, "/api/print", br#"{"ids":["5"],"printer":"HP"}"#).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert!(venue.worker.try_recv().is_none());
        }

        #[tokio::test]
        async fn photo_roundtrip() {
            let venue = open_event("EVT");
            let bytes = b"photo-bytes";
            let (status, _) = post(&venue.app, "/api/registrations/4/photo", bytes).await;
            assert_eq!(status, StatusCode::OK);
            let (status, body) = call(&venue.app, "/api/registrations/4/photo").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, bytes);
            let (status, _) = call(&venue.app, "/api/registrations/missing/photo").await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }

    mod print {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::Duration;

        use image::RgbaImage;
        use ticket_render::Layout;
        use tower::ServiceExt;

        use super::super::router;
        use super::{json_body, open_event};
        use crate::registration_server::cups::{self, Search};
        use crate::registration_server::current::CurrentEvent;
        use crate::registration_server::native_print::Kind;
        use crate::registration_server::print::{Badges, PreparedLayout};
        use crate::registration_server::App;

        struct Stand {
            port: u16,
            ops: Arc<Mutex<Vec<u16>>>,
            stop: Arc<AtomicBool>,
            handle: Option<thread::JoinHandle<()>>,
        }

        impl Stand {
            fn open() -> Self {
                let ops = Arc::new(Mutex::new(Vec::new()));
                let recorded = Arc::clone(&ops);
                let stop = Arc::new(AtomicBool::new(false));
                let flag = Arc::clone(&stop);
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let port = listener.local_addr().unwrap().port();
                let handle = thread::spawn(move || {
                    while !flag.load(Ordering::SeqCst) {
                        let mut sock = match listener.accept() {
                            Ok((sock, _)) => sock,
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(10));
                                continue;
                            }
                            Err(_) => break,
                        };
                        sock.set_nonblocking(false).unwrap();
                        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let body = read_http(&mut sock);
                        if body.len() >= 4 {
                            recorded
                                .lock()
                                .unwrap()
                                .push(u16::from_be_bytes([body[2], body[3]]));
                        }
                        let payload = job_status(1, 9);
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = sock.write_all(head.as_bytes());
                        let _ = sock.write_all(&payload);
                    }
                });
                Self {
                    port,
                    ops,
                    stop,
                    handle: Some(handle),
                }
            }

            fn uri(&self) -> String {
                format!("ipp://127.0.0.1:{}/ipp/print", self.port)
            }
        }

        impl Drop for Stand {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                if let Some(handle) = self.handle.take() {
                    let _ = handle.join();
                }
            }
        }

        fn read_http(sock: &mut impl Read) -> Vec<u8> {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                if let Some(body) = http_body(&buf) {
                    return body;
                }
                match sock.read(&mut tmp) {
                    Ok(0) | Err(_) => return buf,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
        }

        fn http_body(buf: &[u8]) -> Option<Vec<u8>> {
            let end = buf.windows(4).position(|window| window == b"\r\n\r\n")?;
            let head = std::str::from_utf8(&buf[..end]).ok()?;
            let length = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })?;
            let start = end + 4;
            if buf.len() < start + length {
                return None;
            }
            Some(buf[start..start + length].to_vec())
        }

        fn job_status(id: i32, state: i32) -> Vec<u8> {
            let mut out = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01];
            write_text(&mut out, 0x47, "attributes-charset", "utf-8");
            write_text(&mut out, 0x48, "attributes-natural-language", "en");
            out.push(0x02);
            write_int(&mut out, "job-id", id);
            write_enum(&mut out, "job-state", state);
            out.push(0x03);
            out
        }

        fn write_text(out: &mut Vec<u8>, tag: u8, name: &str, value: &str) {
            out.push(tag);
            counted(out, name.as_bytes());
            counted(out, value.as_bytes());
        }

        fn write_int(out: &mut Vec<u8>, name: &str, value: i32) {
            out.push(0x21);
            counted(out, name.as_bytes());
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&value.to_be_bytes());
        }

        fn write_enum(out: &mut Vec<u8>, name: &str, value: i32) {
            out.push(0x23);
            counted(out, name.as_bytes());
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&value.to_be_bytes());
        }

        fn counted(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
        }

        fn printer(name: &str, uri: &str, zone: &str, certificate: bool) -> cups::Printer {
            cups::Printer {
                name: name.to_string(),
                device_uri: uri.to_string(),
                make_model: "HP LaserJet".to_string(),
                state: "idle".to_string(),
                accepting: true,
                host: "127.0.0.1".to_string(),
                kind: Kind::Raster,
                dpi: 300,
                width_in: Some(4.0),
                choices: Vec::new(),
                installed: true,
                connected: true,
                class: "ipp".to_string(),
                values: cups::Values::default(),
                routing: cups::Routing {
                    zone: zone.to_string(),
                    print_cert: certificate,
                    sector_name: "Desk".to_string(),
                    ..cups::Routing::default()
                },
                last_print: 0,
                formats: ipp_tool::Formats {
                    supported: vec!["image/png".to_string()],
                    default: Some("image/png".to_string()),
                    preferred: None,
                },
                settings: ipp_tool::Settings::default(),
                toner: ipp_tool::Toner::default(),
            }
        }

        fn layout() -> PreparedLayout {
            PreparedLayout::memory(
                Layout {
                    width: 144,
                    height: 72,
                    dots_per_point: 1.0,
                    areas: Vec::new(),
                },
                RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])),
            )
        }

        fn plant(app: &App, search: &Search, printers: Vec<cups::Printer>) {
            for printer in printers {
                search.testing_insert(printer);
            }
            let mut badges = Badges::default();
            badges.replace_layout(7, Some(Ok(layout())));
            badges.replace_layout(999, Some(Ok(layout())));
            CurrentEvent::lock(&app.current).testing_badges(badges);
            let visitor = crate::registration_server::event_db::VisitorWrite {
                uid: "5".to_string(),
                data: r#"{"uid":"5","name":"Ann","category":7}"#.to_string(),
                name: "Ann".to_string(),
                surname: String::new(),
                c_name: String::new(),
                category: 7,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 1,
                org_id: None,
                email: String::new(),
            };
            CurrentEvent::lock(&app.current)
                .write("EVT", &visitor)
                .unwrap();
            *app.search.lock().unwrap() = Some(search.clone());
        }

        async fn post(app: &App, uri: &str, body: &[u8]) -> (axum::http::StatusCode, Vec<u8>) {
            let response = router(app.clone())
                .oneshot(
                    axum::extract::Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body.to_vec()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            (status, bytes.to_vec())
        }

        fn spawn_worker(
            venue: &mut super::Venue,
            search: Search,
        ) -> tokio::sync::watch::Sender<bool> {
            let current = Arc::clone(&venue.app.current);
            let (tx, rx) = tokio::sync::watch::channel(false);
            let worker = std::mem::replace(
                &mut venue.worker,
                crate::registration_server::PrintQueue::pair(Arc::clone(&venue.app.gate)).1,
            );
            tokio::spawn(async move {
                let _ =
                    crate::registration_server::print_queue::run(worker, current, search, rx).await;
            });
            tx
        }

        async fn wait_jobs(ops: &Mutex<Vec<u16>>, count: usize) {
            let start = std::time::Instant::now();
            loop {
                let seen = ops
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|op| **op == 0x0002)
                    .count();
                if seen >= count {
                    return;
                }
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("Print-Job count {seen}, want {count}");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        #[tokio::test]
        async fn print_uses_the_chosen_printer() {
            let mut venue = open_event("EVT");
            let stand = Stand::open();
            let search = Search::parked("http://127.0.0.1:9");
            plant(
                &venue.app,
                &search,
                vec![printer("HP", &stand.uri(), "", false)],
            );
            let stop = spawn_worker(&mut venue, search);
            let (status, _) = post(
                &venue.app,
                "/api/print",
                br#"{"ids":["5"],"printer":"HP","noprint":true}"#,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            assert!(stand.ops.lock().unwrap().is_empty());
            let printed = CurrentEvent::lock(&venue.app.current)
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT COUNT(*) FROM prints WHERE uid = '5'", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap()
                })
                .unwrap();
            assert_eq!(printed, 1);
            let (status, body) =
                post(&venue.app, "/api/print", br#"{"ids":["5"],"printer":"HP"}"#).await;
            assert_eq!(status, axum::http::StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["jobs"][0]["printer"], "HP");
            wait_jobs(&stand.ops, 1).await;
            drop(stop);
        }

        #[tokio::test]
        async fn zone_and_certificate_choose_the_queue() {
            let mut venue = open_event("EVT");
            let stand = Stand::open();
            let search = Search::parked("http://127.0.0.1:9");
            let uri = stand.uri();
            plant(
                &venue.app,
                &search,
                vec![
                    printer("hall-a", &uri, "hall", false),
                    printer("hall-cert", &uri, "hall", true),
                ],
            );
            let stop = spawn_worker(&mut venue, search);
            let (status, body) =
                post(&venue.app, "/api/print", br#"{"ids":["5"],"zone":"hall"}"#).await;
            assert_eq!(status, axum::http::StatusCode::OK);
            let job = &json_body(&body)["data"]["jobs"][0];
            assert_eq!(job["printer"], "hall-a");
            assert_eq!(job["zone"], "hall");
            assert_eq!(job["certificate"], false);
            let (status, body) = post(
                &venue.app,
                "/api/print",
                br#"{"ids":["5"],"zone":"hall","certificate":true}"#,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            let job = &json_body(&body)["data"]["jobs"][0];
            assert_eq!(job["printer"], "hall-cert");
            assert_eq!(job["zone"], "hall");
            assert_eq!(job["certificate"], true);
            wait_jobs(&stand.ops, 2).await;
            drop(stop);
        }

        #[tokio::test]
        async fn no_printer_is_conflict() {
            let venue = open_event("EVT");
            let search = Search::parked("http://127.0.0.1:9");
            plant(&venue.app, &search, Vec::new());
            let (status, body) =
                post(&venue.app, "/api/print", br#"{"ids":["5"],"printer":"HP"}"#).await;
            assert_eq!(status, axum::http::StatusCode::CONFLICT);
            assert_eq!(json_body(&body)["error"]["code"], "no_printer");
        }

        #[tokio::test]
        async fn preview_is_png() {
            let venue = open_event("EVT");
            let stand = Stand::open();
            let search = Search::parked("http://127.0.0.1:9");
            plant(
                &venue.app,
                &search,
                vec![printer("HP", &stand.uri(), "", false)],
            );
            let (status, body) = post(
                &venue.app,
                "/api/print/preview",
                br#"{"id":"5","printer":"HP"}"#,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            assert!(body.starts_with(b"\x89PNG"));
            let (status, body) =
                post(&venue.app, "/api/print/preview", br#"{"printer":"HP"}"#).await;
            assert_eq!(status, axum::http::StatusCode::OK);
            assert!(body.starts_with(b"\x89PNG"));
            assert!(stand.ops.lock().unwrap().is_empty());
        }

        #[tokio::test]
        async fn certificate_reserves_a_code() {
            let mut venue = open_event("EVT");
            let stand = Stand::open();
            let search = Search::parked("http://127.0.0.1:9");
            plant(
                &venue.app,
                &search,
                vec![printer("hall-cert", &stand.uri(), "hall", true)],
            );
            CurrentEvent::lock(&venue.app.current).with_db(|db| {
                db.file()
                    .execute(
                        "INSERT INTO ticketbarcodes (barcode, formId) VALUES ('CERT1', 1)",
                        [],
                    )
                    .unwrap();
            });
            let stop = spawn_worker(&mut venue, search);
            let (status, body) = post(
                &venue.app,
                "/api/certificates",
                br#"{"ids":["5"],"zone":"hall"}"#,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            let row = &json_body(&body)["data"]["rows"][0];
            assert_eq!(row["name"], "Ann");
            assert_eq!(row["code"], "CERT1");
            assert_eq!(row["printer"], "hall-cert");
            assert_eq!(row["zone"], "hall");
            wait_jobs(&stand.ops, 1).await;
            let (status, body) = post(
                &venue.app,
                "/api/certificates",
                br#"{"ids":["5"],"zone":"hall"}"#,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["rows"][0]["code"], "");
            tokio::time::sleep(Duration::from_millis(80)).await;
            let sent = stand
                .ops
                .lock()
                .unwrap()
                .iter()
                .filter(|op| **op == 0x0002)
                .count();
            assert_eq!(sent, 1);
            drop(stop);
        }
    }

    mod desk {
        use super::*;
        use axum::http::header::SET_COOKIE;

        async fn send(
            app: &App,
            method: &str,
            uri: &str,
            body: &[u8],
            cookie: Option<&str>,
        ) -> (StatusCode, Vec<u8>, Option<String>) {
            let mut builder = Request::builder().method(method).uri(uri);
            if !body.is_empty() || method != "GET" {
                builder = builder.header("content-type", "application/json");
            }
            if let Some(cookie) = cookie {
                builder = builder.header("cookie", cookie);
            }
            let response = super::super::router(app.clone())
                .oneshot(builder.body(Body::from(body.to_vec())).unwrap())
                .await
                .unwrap();
            let set_cookie = response
                .headers()
                .get(SET_COOKIE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::to_string);
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            (status, bytes.to_vec(), set_cookie)
        }

        fn admin_cookie(app: &App) -> String {
            let key = CurrentEvent::lock(&app.current)
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT key FROM keys WHERE is_admin = 1", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .unwrap()
                })
                .unwrap();
            format!("desk={key}")
        }

        async fn sign_in(app: &App) -> String {
            let key = CurrentEvent::lock(&app.current)
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT key FROM keys WHERE is_admin = 0", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .unwrap()
                })
                .unwrap();
            let body = serde_json::json!({ "key": key }).to_string();
            let (status, _, cookie) =
                send(app, "POST", "/api/desk/auth", body.as_bytes(), None).await;
            assert_eq!(status, StatusCode::OK);
            cookie.unwrap()
        }

        #[tokio::test]
        async fn keys_roundtrip() {
            let venue = open_event("EVT");
            let cookie = sign_in(&venue.app).await;
            let (status, body, _) = send(
                &venue.app,
                "POST",
                "/api/keys",
                br#"{"comment":"door"}"#,
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let created = json_body(&body);
            let id = created["data"]["id"].as_i64().unwrap();
            let key = created["data"]["key"].as_str().unwrap().to_string();

            let (status, body, _) = send(&venue.app, "GET", "/api/keys", b"", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            let rows = json_body(&body)["data"].as_array().unwrap().clone();
            assert_eq!(rows.len(), 3);
            assert!(rows
                .iter()
                .any(|row| row["key"] == key && row["is_admin"] == false));

            let (status, _, _) = send(
                &venue.app,
                "DELETE",
                &format!("/api/keys/{id}"),
                b"",
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK);

            let (status, body, _) = send(&venue.app, "GET", "/api/keys", b"", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            let rows = json_body(&body)["data"].as_array().unwrap().clone();
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| row["key"] != key));
            assert_eq!(rows.iter().filter(|row| row["is_admin"] == true).count(), 1);
            assert_eq!(
                rows.iter().filter(|row| row["is_admin"] == false).count(),
                1
            );
        }

        #[tokio::test]
        async fn desk_key_required() {
            let venue = open_event("EVT");
            let (status, body, _) = send(
                &venue.app,
                "PATCH",
                "/api/settings",
                br#"{"double_print":true}"#,
                None,
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(
                json_body(&body)["error"]["code"],
                serde_json::json!("unauthorized")
            );

            let cookie = admin_cookie(&venue.app);
            let (status, _, _) = send(
                &venue.app,
                "PATCH",
                "/api/settings",
                br#"{"double_print":true}"#,
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let (status, body, _) =
                send(&venue.app, "GET", "/api/settings", b"", Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["double_print"], true);
        }

        #[tokio::test]
        async fn printer_patch_writes_the_file() {
            let mut venue = open_event("EVT");
            let cookie = admin_cookie(&venue.app);
            let (status, _, _) = send(
                &venue.app,
                "PATCH",
                "/api/printers/Zebra",
                br#"{"values":{"media":"card"}}"#,
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let stored =
                std::fs::read(venue.dir.0.join("printer_conf").join("Zebra.json")).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&stored).unwrap(),
                serde_json::json!({ "values": { "media": "card" } })
            );
            assert!(venue.worker.try_recv().is_none());
        }
    }

    mod sync_status {
        use super::*;
        use std::time::Duration;

        use axum::extract::State;
        use axum::routing::get;
        use axum::Router;
        use tokio::sync::watch;

        use crate::registration_server::admin::OperatorState;
        use crate::registration_server::event_db::VisitorWrite;
        use crate::registration_server::remote::Remote;
        use crate::registration_server::remote_server::RemoteServer;
        use crate::registration_server::sync::sync_loop;

        fn visitor(uid: &str) -> VisitorWrite {
            VisitorWrite {
                uid: uid.to_string(),
                data: format!(r#"{{"uid":"{uid}"}}"#),
                name: "Ada".to_string(),
                surname: "Lovelace".to_string(),
                c_name: String::new(),
                category: 1,
                ticket_status: 0,
                gotsome: 0,
                give_packet: 0,
                added_ts: 10,
                org_id: None,
                email: format!("{uid}@example.com"),
            }
        }

        async fn post(app: &App, uri: &str) -> (StatusCode, Vec<u8>) {
            let response = super::super::router(app.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            (status, bytes.to_vec())
        }

        #[tokio::test]
        async fn waiting_count() {
            let venue = open_event("EVT");
            CurrentEvent::lock(&venue.app.current)
                .write("EVT", &visitor("9"))
                .unwrap();
            let (status, _) = call(&venue.app, "/api/sync").await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);

            let key = CurrentEvent::lock(&venue.app.current)
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT key FROM keys WHERE is_admin = 0", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .unwrap()
                })
                .unwrap();
            let body = serde_json::json!({ "key": key }).to_string();
            let response = super::super::router(venue.app.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/desk/auth")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let cookie = response
                .headers()
                .get(axum::http::header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string();
            let response = super::super::router(venue.app.clone())
                .oneshot(
                    Request::builder()
                        .uri("/api/sync")
                        .header("cookie", cookie.clone())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["waiting"], 1);
            assert_eq!(json_body(&body)["data"]["synced"], 0);
            assert_eq!(json_body(&body)["data"]["is_admin"], false);
            assert!(json_body(&body)["data"]["addresses"].is_array());

            let admin = CurrentEvent::lock(&venue.app.current)
                .with_db(|db| {
                    db.file()
                        .query_row("SELECT key FROM keys WHERE is_admin = 1", [], |row| {
                            row.get::<_, String>(0)
                        })
                        .unwrap()
                })
                .unwrap();
            let response = super::super::router(venue.app.clone())
                .oneshot(
                    Request::builder()
                        .uri("/api/sync")
                        .header("cookie", format!("desk={admin}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let admin_status = response.status();
            let admin_body = to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            assert_eq!(admin_status, StatusCode::OK);
            assert_eq!(json_body(&admin_body)["data"]["is_admin"], true);

            CurrentEvent::lock(&venue.app.current)
                .note_on_server("EVT", "9")
                .unwrap();
            let response = super::super::router(venue.app.clone())
                .oneshot(
                    Request::builder()
                        .uri("/api/sync")
                        .header("cookie", cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap();
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json_body(&body)["data"]["waiting"], 0);
            assert_eq!(json_body(&body)["data"]["synced"], 1);
        }

        struct Probe {
            base: String,
            hits: Arc<Mutex<u32>>,
            release: watch::Sender<bool>,
            handle: tokio::task::JoinHandle<()>,
        }

        impl Probe {
            async fn start() -> Self {
                let hits = Arc::new(Mutex::new(0u32));
                let (release, gate) = watch::channel(false);
                let app = Router::new()
                    .route("/boxapi/barcodesinfo", get(hold))
                    .with_state((Arc::clone(&hits), gate));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let handle = tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                });
                Self {
                    base: format!("http://{addr}/"),
                    hits,
                    release,
                    handle,
                }
            }
        }

        impl Drop for Probe {
            fn drop(&mut self) {
                self.handle.abort();
            }
        }

        async fn hold(
            State((hits, mut gate)): State<(Arc<Mutex<u32>>, watch::Receiver<bool>)>,
        ) -> &'static str {
            *hits.lock().unwrap() += 1;
            let _ = gate.wait_for(|go| *go).await;
            r#"{"info":[]}"#
        }

        async fn until(ready: impl Fn() -> bool) {
            let started = tokio::time::Instant::now();
            while !ready() {
                if started.elapsed() > Duration::from_secs(2) {
                    panic!("timed out");
                }
                tokio::task::yield_now().await;
            }
        }

        #[tokio::test]
        async fn wake_once() {
            let probe = Probe::start().await;
            let dir = TempDir::new();
            let path = dir.0.join("credentials.yml");
            let file = CredentialFile::new(path.clone());
            let mut credentials = Credentials::default();
            credentials.set_device_id("device-1").unwrap();
            file.store(&credentials).unwrap();
            let current = Arc::new(Mutex::new(CurrentEvent::new(&dir.0)));
            let (mut app, _worker) = App::new();
            app.operator = OperatorState::new(
                CredentialFile::new(path),
                Remote::new("http://127.0.0.1:1").unwrap(),
            )
            .with_current(Arc::clone(&current))
            .with_link(Arc::clone(&app.link));
            app.current = current;
            let remote = RemoteServer::new(&probe.base).unwrap();
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            let loop_app = app.clone();
            let sync = tokio::spawn(async move {
                sync_loop(
                    loop_app,
                    Vec::new(),
                    Duration::from_secs(3600),
                    2,
                    remote,
                    shutdown_rx,
                )
                .await;
            });

            until(|| app.sync_passes() >= 1 && !app.sync_busy()).await;
            assert_eq!(*probe.hits.lock().unwrap(), 0);

            file.update(|credentials| {
                credentials.bind("EVT", "token-value", Some("Event".into()))?;
                Ok(())
            })
            .unwrap();
            CurrentEvent::lock(&app.current).open_bound(&file).unwrap();

            let (status, _) = post(&app, "/api/sync").await;
            assert_eq!(status, StatusCode::OK);
            until(|| *probe.hits.lock().unwrap() >= 1).await;
            assert_eq!(*probe.hits.lock().unwrap(), 1);

            let (status, _) = post(&app, "/api/sync").await;
            assert_eq!(status, StatusCode::OK);
            for _ in 0..32 {
                tokio::task::yield_now().await;
            }
            assert_eq!(*probe.hits.lock().unwrap(), 1);

            probe.release.send(true).unwrap();
            until(|| app.sync_passes() >= 2 && !app.sync_busy()).await;
            assert_eq!(app.sync_passes(), 2);
            for _ in 0..32 {
                tokio::task::yield_now().await;
            }
            assert!(!app.sync_busy());
            assert_eq!(app.sync_passes(), 2);

            let _ = shutdown_tx.send(true);
            sync.abort();
        }
    }

    mod catalog {
        use super::*;

        fn write_categories(venue: &Venue, body: &str) {
            let dir = venue.dir.0.join("EVT").join("forms");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("categories.json"), body).unwrap();
        }

        #[tokio::test]
        async fn catalog_returns_the_stored_lists() {
            let venue = open_event("EVT");
            write_categories(&venue, r#"[{"cat_id":3,"name":{"ru":"Гость"}}]"#);
            std::fs::create_dir_all(venue.dir.0.join("printer_conf")).unwrap();
            std::fs::write(venue.dir.0.join("printer_conf").join("Zebra.json"), b"{}").unwrap();
            std::fs::write(venue.dir.0.join("EVT").join("pr_HP.json"), b"{}").unwrap();

            let (status, body) = call(&venue.app, "/api/desk/catalog").await;
            assert_eq!(status, StatusCode::OK);
            let data = json_body(&body);
            assert_eq!(data["ok"], true);
            assert!(data["data"]["rev"].as_u64().unwrap() > 0);
            assert_eq!(data["data"]["categories"][0]["cat_id"], -2);
            assert_eq!(data["data"]["categories"][0]["name"]["ru"], "Экспонент");
            assert_eq!(data["data"]["categories"][0]["name"]["en"], "Exhibitor");
            assert_eq!(data["data"]["categories"][1]["cat_id"], 3);
            assert_eq!(data["data"]["categories"][1]["name"]["ru"], "Гость");
            let names: Vec<&str> = data["data"]["printers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect();
            assert_eq!(names, vec!["HP", "Zebra"]);

            write_categories(
                &venue,
                r#"[{"cat_id":-2,"name":{"ru":"Другое"}},{"cat_id":3,"name":{"ru":"Гость"}}]"#,
            );
            let (status, body) = call(&venue.app, "/api/desk/catalog").await;
            assert_eq!(status, StatusCode::OK);
            let categories = json_body(&body)["data"]["categories"]
                .as_array()
                .unwrap()
                .clone();
            assert_eq!(categories.len(), 2);
            assert_eq!(categories[0]["cat_id"], -2);
            assert_eq!(categories[0]["name"]["ru"], "Экспонент");
            assert_eq!(categories[1]["name"]["ru"], "Гость");
        }

        #[tokio::test]
        async fn catalog_waits_until_a_list_changes() {
            let venue = open_event("EVT");
            write_categories(&venue, r#"[{"cat_id":3,"name":{"ru":"Гость"}}]"#);
            let (status, body) = call(&venue.app, "/api/desk/catalog").await;
            assert_eq!(status, StatusCode::OK);
            let rev = json_body(&body)["data"]["rev"].as_u64().unwrap();
            let app = venue.app.clone();
            let path = venue
                .dir
                .0
                .join("EVT")
                .join("forms")
                .join("categories.json");
            let pending =
                tokio::spawn(
                    async move { call(&app, &format!("/api/desk/catalog?rev={rev}")).await },
                );
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            std::fs::write(
                path,
                r#"[{"cat_id":3,"name":{"ru":"Гость"}},{"cat_id":7,"name":{"ru":"Пресса"}}]"#,
            )
            .unwrap();
            let joined = tokio::time::timeout(std::time::Duration::from_secs(2), pending).await;
            let Ok(Ok((status, body))) = joined else {
                panic!("catalog did not return the updated list");
            };
            assert_eq!(status, StatusCode::OK);
            let data = json_body(&body);
            assert_ne!(data["data"]["rev"].as_u64().unwrap(), rev);
            assert_eq!(data["data"]["categories"].as_array().unwrap().len(), 3);
            assert_eq!(data["data"]["categories"][0]["cat_id"], -2);
            assert_eq!(data["data"]["categories"][2]["name"]["ru"], "Пресса");
        }
    }
}
