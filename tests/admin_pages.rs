use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use rust_reg::registration_server::{api_router, CredentialFile, OperatorState, Remote};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower::ServiceExt;

mod login_failure {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rust-reg-admin-fail-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn credentials(device_id: Option<&str>) -> (Scratch, CredentialFile) {
        let scratch = Scratch::new();
        let file = CredentialFile::new(scratch.0.join("credentials.yml"));
        if let Some(device_id) = device_id {
            file.update(|credentials| {
                credentials.set_device_id(device_id)?;
                Ok(())
            })
            .unwrap();
        }
        (scratch, file)
    }

    fn login_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/login")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn call(state: OperatorState, request: Request<Body>) -> axum::response::Response {
        api_router(state).oneshot(request).await.unwrap()
    }

    async fn error_body(response: axum::response::Response) -> (StatusCode, Value, bool) {
        let status = response.status();
        let cookie = response.headers().get("set-cookie").is_some();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes).unwrap();
        (status, body, cookie)
    }

    async fn rejected_password() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/auth/login",
                    post(|| async { Json(json!({ "status": 1 })) }),
                ),
            )
            .await
            .unwrap();
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn missing_device_is_no_device_id() {
        let (_scratch, file) = credentials(None);
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let response = call(
            OperatorState::new(file, remote),
            login_request(r#"{"login":"a@b.c","password":"secret"}"#),
        )
        .await;
        let (status, body, cookie) = error_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "no_device_id" }, "timeout_secs": 10 })
        );
        assert!(!cookie);
    }

    #[tokio::test]
    async fn bad_password_is_invalid_cred() {
        let base = rejected_password().await;
        let (_scratch, file) = credentials(Some("device-1"));
        let remote = Remote::new(&base).unwrap();
        let response = call(
            OperatorState::new(file, remote),
            login_request(r#"{"login":"a@b.c","password":"bad"}"#),
        )
        .await;
        let (status, body, _) = error_body(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "invalid_cred");
        assert_eq!(body["ok"], false);
    }

    #[tokio::test]
    async fn down_remote_is_no_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                drop(socket);
            }
        });
        let (_scratch, file) = credentials(Some("device-1"));
        let remote =
            Remote::with_timeout(&format!("http://{addr}/"), Duration::from_secs(2)).unwrap();
        let response = call(
            OperatorState::new(file, remote),
            login_request(r#"{"login":"a@b.c","password":"secret"}"#),
        )
        .await;
        let (status, body, _) = error_body(response).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["code"], "no_connection");
    }

    #[tokio::test]
    async fn malformed_login_is_bad_request() {
        let (_scratch, file) = credentials(None);
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let response = call(OperatorState::new(file, remote), login_request("not-json")).await;
        let (status, body, _) = error_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "bad_request" }, "timeout_secs": 10 })
        );
    }

    #[tokio::test]
    async fn failed_login_sets_no_cookie() {
        let (_scratch, file) = credentials(None);
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let state = OperatorState::new(file, remote);
        let response = call(
            state.clone(),
            login_request(r#"{"login":"a@b.c","password":"secret"}"#),
        )
        .await;
        let (status, _, cookie) = error_body(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!cookie);

        let events = Request::builder()
            .uri("/api/events")
            .body(Body::empty())
            .unwrap();
        let response = call(state, events).await;
        let (status, body, cookie) = error_body(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "session_expired" } })
        );
        assert!(!cookie);
    }

    #[derive(Clone)]
    struct Hold {
        hits: Arc<AtomicUsize>,
        release: watch::Receiver<bool>,
    }

    async fn held_login(State(hold): State<Hold>) -> Json<Value> {
        hold.hits.fetch_add(1, Ordering::SeqCst);
        let mut release = hold.release.clone();
        loop {
            if *release.borrow_and_update() {
                break;
            }
            if release.changed().await.is_err() {
                break;
            }
        }
        Json(json!({ "status": 1 }))
    }

    #[tokio::test]
    async fn login_in_progress_skips_remote() {
        let hits = Arc::new(AtomicUsize::new(0));
        let (release_tx, release_rx) = watch::channel(false);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hold = Hold {
            hits: Arc::clone(&hits),
            release: release_rx,
        };
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/auth/login", post(held_login))
                    .with_state(hold),
            )
            .await
            .unwrap();
        });

        let (_scratch, file) = credentials(Some("device-1"));
        let remote =
            Remote::with_timeout(&format!("http://{addr}/"), Duration::from_secs(5)).unwrap();
        let state = OperatorState::new(file, remote);
        let app = api_router(state);
        let first = {
            let app = app.clone();
            tokio::spawn(async move {
                app.oneshot(login_request(r#"{"login":"a@b.c","password":"secret"}"#))
                    .await
                    .unwrap()
            })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while hits.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("stand-in saw the first login");

        let second = app
            .oneshot(login_request(r#"{"login":"a@b.c","password":"secret"}"#))
            .await
            .unwrap();
        let (status, body, cookie) = error_body(second).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "login_in_progress" }, "timeout_secs": 10 })
        );
        assert!(!cookie);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        release_tx.send(true).unwrap();
        let first = tokio::time::timeout(Duration::from_secs(2), first)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rust-reg-admin-ok-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn credentials(device_id: &str) -> (Scratch, CredentialFile) {
    let scratch = Scratch::new();
    let file = CredentialFile::new(scratch.0.join("credentials.yml"));
    file.update(|credentials| {
        credentials.set_device_id(device_id)?;
        Ok(())
    })
    .unwrap();
    (scratch, file)
}

fn login_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/login")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"login":"a@b.c","password":"secret"}"#.to_string(),
        ))
        .unwrap()
}

#[derive(Clone)]
struct Stand {
    hits: Arc<AtomicUsize>,
    synch_hits: Arc<AtomicUsize>,
    login_hits: Arc<AtomicUsize>,
    barcode_hits: Arc<AtomicUsize>,
    logged_out: bool,
    bind_status: StatusCode,
    release: Option<watch::Receiver<bool>>,
}

struct StandIn {
    base: String,
    hits: Arc<AtomicUsize>,
    synch_hits: Arc<AtomicUsize>,
    login_hits: Arc<AtomicUsize>,
    barcode_hits: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl StandIn {
    async fn start(logged_out: bool) -> Self {
        Self::open(logged_out, StatusCode::OK).await
    }

    async fn hold() -> (Self, watch::Sender<bool>) {
        let (release_tx, release_rx) = watch::channel(false);
        let stand = Self::serve(false, StatusCode::OK, Some(release_rx)).await;
        (stand, release_tx)
    }

    async fn open(logged_out: bool, bind_status: StatusCode) -> Self {
        Self::serve(logged_out, bind_status, None).await
    }

    async fn serve(
        logged_out: bool,
        bind_status: StatusCode,
        release: Option<watch::Receiver<bool>>,
    ) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let synch_hits = Arc::new(AtomicUsize::new(0));
        let login_hits = Arc::new(AtomicUsize::new(0));
        let barcode_hits = Arc::new(AtomicUsize::new(0));
        let stand = Stand {
            hits: Arc::clone(&hits),
            synch_hits: Arc::clone(&synch_hits),
            login_hits: Arc::clone(&login_hits),
            barcode_hits: Arc::clone(&barcode_hits),
            logged_out,
            bind_status,
            release,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let app = Router::new()
                .route("/auth/login", post(stand_login))
                .route("/", axum::routing::get(stand_session))
                .route("/boxapi/expos/ru", axum::routing::get(stand_events))
                .route(
                    "/profile/synchdev/{device_id}/{event_id}",
                    axum::routing::get(stand_synchdev),
                )
                .route("/boxapi/barcodesinfo", axum::routing::get(stand_barcodes))
                .with_state(stand);
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base: format!("http://{addr}/"),
            hits,
            synch_hits,
            login_hits,
            barcode_hits,
            handle,
        }
    }
}

async fn stand_login(State(stand): State<Stand>) -> Json<Value> {
    stand.login_hits.fetch_add(1, Ordering::SeqCst);
    Json(json!({ "temptoken": "temp-1" }))
}

async fn stand_session() -> impl axum::response::IntoResponse {
    (
        StatusCode::OK,
        [(
            axum::http::header::SET_COOKIE,
            "JSESSIONID=sess-1; Path=/; HttpOnly",
        )],
        "ok",
    )
}

async fn stand_synchdev(State(stand): State<Stand>) -> impl axum::response::IntoResponse {
    stand.synch_hits.fetch_add(1, Ordering::SeqCst);
    if let Some(release) = stand.release.clone() {
        let mut release = release;
        loop {
            if *release.borrow_and_update() {
                break;
            }
            if release.changed().await.is_err() {
                break;
            }
        }
    }
    if stand.bind_status != StatusCode::OK {
        return (stand.bind_status, Json(json!({})));
    }
    (StatusCode::OK, Json(json!({ "token": "project-token" })))
}

async fn stand_barcodes(State(stand): State<Stand>) -> Json<Value> {
    stand.barcode_hits.fetch_add(1, Ordering::SeqCst);
    Json(json!({ "info": [] }))
}

async fn stand_events(State(stand): State<Stand>) -> Json<Value> {
    stand.hits.fetch_add(1, Ordering::SeqCst);
    if stand.logged_out {
        return Json(json!({
            "status": { "errors": [{ "msg": "You are not logged in!" }] }
        }));
    }
    Json(json!({
        "list": [{
            "uniqueId": "5245081",
            "name": { "ru": { "str": "TechCrunch" } }
        }]
    }))
}

async fn post_login(state: OperatorState) -> axum::response::Response {
    api_router(state).oneshot(login_request()).await.unwrap()
}

fn operator_cookie(response: &axum::response::Response) -> String {
    let raw = response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(raw.contains("HttpOnly"), "{raw}");
    assert!(raw.contains("SameSite=Lax"), "{raw}");
    assert!(raw.contains("Path=/"), "{raw}");
    let pair = raw.split(';').next().unwrap().trim();
    let (name, value) = pair.split_once('=').unwrap();
    assert_eq!(name, "operator");
    format!("operator={value}")
}

async fn body_of(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

mod login_success {
    use super::*;

    #[tokio::test]
    async fn login_sets_cookie() {
        let stand = StandIn::start(false).await;
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new(&stand.base).unwrap();
        let response = post_login(OperatorState::new(file, remote)).await;
        let cookie = operator_cookie(&response);
        assert!(cookie.starts_with("operator="));
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "ok": true, "data": { "timeout_secs": 10 } }));
    }

    #[tokio::test]
    async fn second_login_replaces_session() {
        let stand = StandIn::start(false).await;
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new(&stand.base).unwrap();
        let state = OperatorState::new(file, remote);
        let first = post_login(state.clone()).await;
        let first_cookie = operator_cookie(&first);
        let _second = post_login(state.clone()).await;
        let response = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, first_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "session_expired" } })
        );
    }
}

mod events {
    use super::*;

    async fn login(logged_out: bool) -> (StandIn, OperatorState, String) {
        let stand = StandIn::start(logged_out).await;
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new(&stand.base).unwrap();
        let state = OperatorState::new(file, remote);
        let response = post_login(state.clone()).await;
        let cookie = operator_cookie(&response);
        assert_eq!(response.status(), StatusCode::OK);
        (stand, state, cookie)
    }

    #[tokio::test]
    async fn events_shape() {
        let (stand, state, cookie) = login(false).await;
        let response = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["current"], Value::Null);
        assert_eq!(body["data"]["events"][0]["id"], "5245081");
        assert_eq!(body["data"]["events"][0]["name"], "TechCrunch");
        assert_eq!(stand.hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn events_current_from_file() {
        let stand = StandIn::start(false).await;
        let (_scratch, file) = credentials("device-1");
        file.update(|credentials| {
            credentials.bind("5245081", "project-token", Some("TechCrunch".into()))?;
            Ok(())
        })
        .unwrap();
        let remote = Remote::new(&stand.base).unwrap();
        let state = OperatorState::new(file, remote);
        let response = post_login(state.clone()).await;
        let cookie = operator_cookie(&response);
        let response = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["current"],
            json!({ "id": "5245081", "name": "TechCrunch" })
        );
    }

    #[tokio::test]
    async fn events_unknown_cookie() {
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let response = api_router(OperatorState::new(file, remote))
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, "operator=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "session_expired" } })
        );
    }

    #[tokio::test]
    async fn events_dead_remote_cookie() {
        let (stand, state, cookie) = login(true).await;
        let response = api_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "session_expired" } })
        );
        assert_eq!(stand.hits.load(Ordering::SeqCst), 1);

        let again = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, _) = body_of(again).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(stand.hits.load(Ordering::SeqCst), 1);
    }
}

mod network {
    use super::*;

    #[tokio::test]
    async fn network_after_two_pings() {
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let state = OperatorState::new(file, remote);
        state.link.record(true, Duration::from_millis(50));
        state.link.record(true, Duration::from_millis(80));
        let response = api_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/network")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["samples"], 2);
        assert_eq!(body["data"]["offline"], false);
        assert_eq!(body["data"]["quality"], "good");
        assert_eq!(body["data"]["timeout_secs"], 5);

        let login = post_login(state).await;
        let (_, login_body) = body_of(login).await;
        assert_eq!(login_body["timeout_secs"], 5);
    }

    #[tokio::test]
    async fn network_hides_quality_until_two_samples() {
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let state = OperatorState::new(file, remote);
        state.link.record(false, Duration::from_secs(4));
        let response = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/network")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["samples"], 1);
        assert_eq!(body["data"]["quality"], Value::Null);
        assert_eq!(body["data"]["offline"], false);
        assert_eq!(body["data"]["timeout_secs"], 10);
    }
}

mod select_event {
    use super::*;

    pub(crate) async fn logged_in(
        bind_status: StatusCode,
    ) -> (StandIn, OperatorState, String, Scratch, PathBuf) {
        let stand = StandIn::open(false, bind_status).await;
        let (scratch, file) = credentials("device-1");
        let path = file.path().to_path_buf();
        let remote = Remote::new(&stand.base).unwrap();
        let state = OperatorState::new(file, remote);
        let response = post_login(state.clone()).await;
        let cookie = operator_cookie(&response);
        assert_eq!(response.status(), StatusCode::OK);
        (stand, state, cookie, scratch, path)
    }

    pub(crate) fn select_request(cookie: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/events/select")
            .header(axum::http::header::COOKIE, cookie)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn select_stores_token() {
        let (_stand, state, cookie, _scratch, path) = logged_in(StatusCode::OK).await;
        let response = api_router(state)
            .oneshot(select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        assert!(response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .is_none());
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["id"], "5245081");
        assert_eq!(body["data"]["name"], "TechCrunch");
        assert_eq!(body["data"]["token_stored"], true);
        assert!(!body.to_string().contains("project-token"));
        let stored = CredentialFile::new(&path).load().unwrap();
        assert_eq!(stored.event_id(), Some("5245081"));
        assert_eq!(stored.event_name(), Some("TechCrunch"));
        assert_eq!(stored.project_token(), Some("project-token"));
        assert_eq!(body["data"]["timeout_secs"], 10);
    }

    #[tokio::test]
    async fn select_does_not_probe() {
        let (stand, state, cookie, _scratch, _path) = logged_in(StatusCode::OK).await;
        let response = api_router(state)
            .oneshot(select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(stand.synch_hits.load(Ordering::SeqCst) >= 1);
        assert_eq!(stand.barcode_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn select_timeout_follows_the_link() {
        let (_stand, state, cookie, _scratch, _path) = logged_in(StatusCode::OK).await;
        state.link.record(true, Duration::from_millis(40));
        state.link.record(true, Duration::from_millis(80));
        let response = api_router(state)
            .oneshot(select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["timeout_secs"], 5);
    }

    #[tokio::test]
    async fn select_keeps_session() {
        let (_stand, state, cookie, _scratch, _path) = logged_in(StatusCode::OK).await;
        let selected = api_router(state.clone())
            .oneshot(select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(selected.status(), StatusCode::OK);
        let events = api_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(events).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["events"][0]["id"], "5245081");
        let binding = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/binding")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(binding).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["token_stored"], true);
        assert_eq!(body["data"]["id"], "5245081");
    }

    #[tokio::test]
    async fn select_bind_failure_keeps_file() {
        let (_stand, state, cookie, _scratch, path) =
            logged_in(StatusCode::INTERNAL_SERVER_ERROR).await;
        let response = api_router(state.clone())
            .oneshot(select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "unknown_error" }, "timeout_secs": 10 })
        );
        let stored = CredentialFile::new(&path).load().unwrap();
        assert_eq!(stored.project_token(), None);
        let again = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn select_in_progress_skips_remote() {
        let (stand, release) = StandIn::hold().await;
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new(&stand.base).unwrap();
        let state = OperatorState::new(file, remote);
        let response = post_login(state.clone()).await;
        let cookie = operator_cookie(&response);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(stand.login_hits.load(Ordering::SeqCst), 1);

        let app = api_router(state);
        let first = {
            let app = app.clone();
            let cookie = cookie.clone();
            tokio::spawn(async move {
                app.oneshot(select_request(
                    &cookie,
                    r#"{"id":"5245081","name":"TechCrunch"}"#,
                ))
                .await
                .unwrap()
            })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while stand.synch_hits.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("stand-in saw the first select");

        let second = app
            .clone()
            .oneshot(select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(second).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "select_in_progress" }, "timeout_secs": 10 })
        );
        assert_eq!(stand.synch_hits.load(Ordering::SeqCst), 1);

        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(listed).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "in_progress" } })
        );
        assert_eq!(stand.hits.load(Ordering::SeqCst), 0);

        let again = app.clone().oneshot(login_request()).await.unwrap();
        let (status, body) = body_of(again).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "login_in_progress" }, "timeout_secs": 10 })
        );
        assert_eq!(stand.login_hits.load(Ordering::SeqCst), 1);

        release.send(true).unwrap();
        let first = tokio::time::timeout(Duration::from_secs(2), first)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(stand.synch_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn select_empty_id() {
        let (_stand, state, cookie, _scratch, _path) = logged_in(StatusCode::OK).await;
        let response = api_router(state.clone())
            .oneshot(select_request(
                &cookie,
                r#"{"id":"  ","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "bad_request" }, "timeout_secs": 10 })
        );
        let still = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(still.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn keys_empty() {
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let response = api_router(OperatorState::new(file, remote))
            .oneshot(
                Request::builder()
                    .uri("/api/keys")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "ok": true, "data": [] }));
    }

    #[tokio::test]
    async fn create_key_not_implemented() {
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new("http://127.0.0.1:1").unwrap();
        let state = OperatorState::new(file, remote);
        let response = api_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/keys")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "keys_later" } })
        );
        let printed = api_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/keys/print")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(printed).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "keys_later" } })
        );
    }
}

mod operator_session {
    use super::*;

    async fn logged_in(ttl: Duration) -> (StandIn, OperatorState, String) {
        let stand = StandIn::open(false, StatusCode::OK).await;
        let (_scratch, file) = credentials("device-1");
        let remote = Remote::new(&stand.base).unwrap();
        let state = OperatorState::new(file, remote).with_session_ttl(ttl);
        let response = post_login(state.clone()).await;
        let cookie = operator_cookie(&response);
        (stand, state, cookie)
    }

    fn events(cookie: &str) -> Request<Body> {
        Request::builder()
            .uri("/api/events")
            .header(axum::http::header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn session_expires_without_a_ping() {
        let (_stand, state, cookie) = logged_in(Duration::from_millis(80)).await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        let response = api_router(state).oneshot(events(&cookie)).await.unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "session_expired" } })
        );
    }

    #[tokio::test]
    async fn session_ping_resets_the_deadline() {
        let (_stand, state, cookie) = logged_in(Duration::from_millis(300)).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let ping = api_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session")
                    .header(axum::http::header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ping.status(), StatusCode::OK);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let listed = api_router(state.clone())
            .oneshot(events(&cookie))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let expired = api_router(state).oneshot(events(&cookie)).await.unwrap();
        let (status, body) = body_of(expired).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "session_expired" } })
        );
    }
}

mod serve_admin {
    use super::*;
    use rust_reg::registration_server::{serve, App};

    const INDEX: &str = "<!doctype html><h1>mounted-index</h1>";

    struct Running {
        addr: std::net::SocketAddr,
        shutdown: watch::Sender<bool>,
        server:
            Option<tokio::task::JoinHandle<Result<(), rust_reg::registration_server::ServeError>>>,
        _stand: StandIn,
        _scratch: Scratch,
    }

    async fn started() -> Running {
        let stand = StandIn::start(false).await;
        let scratch = Scratch::new();
        let dist = scratch.0.join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), INDEX.as_bytes()).unwrap();
        std::fs::write(dist.join("assets/app.js"), b"console.log(1)").unwrap();
        let file = CredentialFile::new(scratch.0.join("credentials.yml"));
        file.update(|credentials| {
            credentials.set_device_id("device-1")?;
            Ok(())
        })
        .unwrap();
        let remote = Remote::new(&stand.base).unwrap();
        let (mut app, worker) = App::new();
        app.dist = dist;
        app.operator = OperatorState::new(file, remote).with_link(std::sync::Arc::clone(&app.link));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let server = tokio::spawn(serve(listener, app, worker, shutdown_rx));
        Running {
            addr,
            shutdown,
            server: Some(server),
            _stand: stand,
            _scratch: scratch,
        }
    }

    async fn stop(mut running: Running) {
        running.shutdown.send(true).unwrap();
        let server = running.server.take().unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn serve_index() {
        let running = started().await;
        let client = reqwest::Client::new();
        let root = client
            .get(format!("http://{}/", running.addr))
            .send()
            .await
            .unwrap();
        assert_eq!(root.status(), reqwest::StatusCode::OK);
        assert_eq!(root.text().await.unwrap(), INDEX);
        let events = client
            .get(format!("http://{}/events", running.addr))
            .send()
            .await
            .unwrap();
        assert_eq!(events.status(), reqwest::StatusCode::OK);
        assert_eq!(events.text().await.unwrap(), INDEX);
        stop(running).await;
    }

    #[tokio::test]
    async fn serve_asset_304() {
        let running = started().await;
        let client = reqwest::Client::new();
        let url = format!("http://{}/assets/app.js", running.addr);
        let first = client.get(&url).send().await.unwrap();
        assert_eq!(first.status(), reqwest::StatusCode::OK);
        let tag = first
            .headers()
            .get(reqwest::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(first.text().await.unwrap(), "console.log(1)");
        let again = client
            .get(url)
            .header(reqwest::header::IF_NONE_MATCH, tag)
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), reqwest::StatusCode::NOT_MODIFIED);
        assert!(again.text().await.unwrap().is_empty());
        stop(running).await;
    }

    #[tokio::test]
    async fn serve_login() {
        let running = started().await;
        let response = reqwest::Client::new()
            .post(format!("http://{}/api/login", running.addr))
            .json(&json!({"login": "a@b.c", "password": "secret"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let cookie = response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.starts_with("operator="), "{cookie}");
        assert!(cookie.contains("HttpOnly"), "{cookie}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["ok"], true);
        stop(running).await;
    }

    #[tokio::test]
    async fn health_still_ok() {
        let running = started().await;
        let health = reqwest::get(format!("http://{}/health", running.addr))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(health, "{\"status\":\"ok\"}");
        stop(running).await;
    }

    #[tokio::test]
    async fn api_is_not_index() {
        let running = started().await;
        let response = reqwest::get(format!("http://{}/api/missing", running.addr))
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let text = response.text().await.unwrap();
        assert!(!text.contains("mounted-index"), "{text}");
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            body,
            json!({ "ok": false, "error": { "code": "not_found" } })
        );
        stop(running).await;
    }
}

mod select_opens_event {
    use super::*;
    use rusqlite::Connection;
    use rust_reg::registration_server::event_db::DB_FILE;
    use rust_reg::registration_server::{CurrentEvent, IndexState, VisitorWrite};

    fn visitor(uid: &str) -> VisitorWrite {
        VisitorWrite {
            uid: uid.to_string(),
            data: format!(r#"{{"uid":"{uid}"}}"#),
            name: "запись".to_string(),
            surname: "выбор".to_string(),
            c_name: String::new(),
            category: 7,
            ticket_status: 0,
            gotsome: 0,
            give_packet: 0,
            added_ts: 10,
            org_id: None,
            email: format!("{uid}@example.com"),
        }
    }

    fn file_has(path: &std::path::Path, uid: &str) -> bool {
        let conn = Connection::open(path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mem_u WHERE uid = ?1", [uid], |row| {
                row.get(0)
            })
            .unwrap();
        count > 0
    }

    fn open_with_visitor(state: &OperatorState, id: &str, uid: &str) {
        let current = CurrentEvent::lock(state.current());
        current.switch_to(id).unwrap();
        current.write(id, &visitor(uid)).unwrap();
    }

    #[tokio::test]
    async fn select_opens_the_file() {
        let (_stand, state, cookie, scratch, _path) = select_event::logged_in(StatusCode::OK).await;
        let response = api_router(state.clone())
            .oneshot(select_event::select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["token_stored"], true);
        assert!(!body.to_string().contains("project-token"));
        assert!(scratch.0.join("5245081").join(DB_FILE).exists());
        let current = CurrentEvent::lock(state.current());
        assert_eq!(current.event_id().as_deref(), Some("5245081"));
        assert_eq!(current.index_state().unwrap(), Some(IndexState::Ready));
        drop(current);
        let binding = api_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/binding")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_of(binding).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["token_stored"], true);
    }

    #[tokio::test]
    async fn select_replaces_the_previous_file() {
        let (_stand, state, cookie, scratch, _path) = select_event::logged_in(StatusCode::OK).await;
        let first = api_router(state.clone())
            .oneshot(select_event::select_request(
                &cookie,
                r#"{"id":"AAA","name":"First"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        open_with_visitor(&state, "AAA", "1");
        let second = api_router(state.clone())
            .oneshot(select_event::select_request(
                &cookie,
                r#"{"id":"BBB","name":"Second"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let current = CurrentEvent::lock(state.current());
        assert_eq!(current.event_id().as_deref(), Some("BBB"));
        assert_eq!(current.loaded_count().unwrap(), 0);
        drop(current);
        assert!(file_has(&scratch.0.join("AAA").join(DB_FILE), "1"));
        assert!(!file_has(&scratch.0.join("BBB").join(DB_FILE), "1"));
    }

    #[tokio::test]
    async fn select_bind_failure_does_not_open() {
        let (stand, state, cookie, scratch, _path) =
            select_event::logged_in(StatusCode::INTERNAL_SERVER_ERROR).await;
        open_with_visitor(&state, "AAA", "1");
        let response = api_router(state.clone())
            .oneshot(select_event::select_request(
                &cookie,
                r#"{"id":"5245081","name":"TechCrunch"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "unknown_error");
        assert!(!scratch.0.join("5245081").join(DB_FILE).exists());
        let current = CurrentEvent::lock(state.current());
        assert_eq!(current.event_id().as_deref(), Some("AAA"));
        assert_eq!(current.loaded_count().unwrap(), 1);
        assert!(stand.synch_hits.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn select_rejects_a_bad_id() {
        let (stand, state, cookie, scratch, path) = select_event::logged_in(StatusCode::OK).await;
        open_with_visitor(&state, "AAA", "1");
        let before = std::fs::read(&path).unwrap();
        let response = api_router(state.clone())
            .oneshot(select_event::select_request(
                &cookie,
                r#"{"id":"../x","name":"Nope"}"#,
            ))
            .await
            .unwrap();
        let (status, body) = body_of(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "bad_request");
        assert_eq!(stand.synch_hits.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let current = CurrentEvent::lock(state.current());
        assert_eq!(current.event_id().as_deref(), Some("AAA"));
        assert_eq!(current.loaded_count().unwrap(), 1);
        assert!(!scratch.0.join("x").exists());
    }
}
