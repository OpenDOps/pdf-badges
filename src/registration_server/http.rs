use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::registration_server::gate::Gate;
use crate::registration_server::print_queue::{EnqueueError, PrintJob};
use crate::registration_server::store::Registration;
use crate::registration_server::{App, ServeError};

pub fn router(app: App) -> Router {
    let gate = Arc::clone(&app.gate);
    let dist = app.dist.clone();
    let operator = app.operator.clone();
    let pages = Router::new()
        .route("/health", get(health))
        .route("/registrations/{id}", get(lookup))
        .route("/print", post(enqueue_print))
        .route("/ws", get(ws))
        .route("/", get(index).head(index))
        .route("/events", get(index).head(index))
        .with_state(app);
    crate::registration_server::admin::api_router(operator)
        .merge(pages)
        .fallback(move |request: Request| {
            let dist = dist.clone();
            async move {
                let path = request.uri().path();
                if path == "/api" || path.starts_with("/api/") {
                    return api_missing().await;
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
    let _guard = gate.enter_request();
    let response = next.run(request).await;
    log::debug!("{method} {path} {}", response.status());
    response
}

async fn index(State(app): State<App>, request: Request) -> Response {
    crate::registration_server::admin::serve_index(&app.dist, request).await
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
