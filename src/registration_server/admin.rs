use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH,
    LAST_MODIFIED,
};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;

pub struct FileMeta {
    pub mtime_secs: u64,
    pub len: u64,
}

pub enum Freshness {
    Full,
    NotModified,
}

pub fn etag(mtime_secs: u64, len: u64) -> String {
    format!("\"{mtime_secs}-{len}\"")
}

pub fn conditional_get(
    meta: &FileMeta,
    if_none_match: Option<&str>,
    if_modified_since: Option<&str>,
) -> Freshness {
    let tag = etag(meta.mtime_secs, meta.len);
    if let Some(header) = if_none_match {
        return if none_match_hits(&tag, header) {
            Freshness::NotModified
        } else {
            Freshness::Full
        };
    }
    if let Some(since) = if_modified_since {
        if let Ok(parsed) = httpdate::parse_http_date(since) {
            let since_secs = system_time_secs(parsed);
            if since_secs >= meta.mtime_secs {
                return Freshness::NotModified;
            }
        }
    }
    Freshness::Full
}

pub fn safe_public_file(root: &Path, url_path: &str) -> Option<PathBuf> {
    if url_path.contains('\\') {
        return None;
    }
    let decoded = percent_decode(url_path)?;
    if decoded.contains('\\') || decoded.is_empty() || decoded == "/" || decoded.ends_with('/') {
        return None;
    }
    let relative = decoded.trim_start_matches('/');
    if relative.is_empty() {
        return None;
    }
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
    }
    let root = root.canonicalize().ok()?;
    let file = root.join(relative).canonicalize().ok()?;
    if !file.starts_with(&root) || !file.is_file() {
        return None;
    }
    Some(file)
}

pub fn admin_router(root: PathBuf) -> Router {
    Router::new().fallback(static_file).with_state(root)
}

fn none_match_hits(etag: &str, header: &str) -> bool {
    header.split(',').any(|part| {
        let part = part.trim();
        part == "*" || part == etag
    })
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn system_time_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn last_modified(mtime_secs: u64) -> String {
    httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(mtime_secs))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn file_meta(path: &Path) -> Option<FileMeta> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(FileMeta {
        mtime_secs: system_time_secs(modified),
        len: meta.len(),
    })
}

async fn static_file(State(root): State<PathBuf>, request: Request) -> Response {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Some(path) = safe_public_file(&root, request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(meta) = file_meta(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let tag = etag(meta.mtime_secs, meta.len);
    let modified = last_modified(meta.mtime_secs);
    let if_none_match = request
        .headers()
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    let if_modified_since = request
        .headers()
        .get(IF_MODIFIED_SINCE)
        .and_then(|value| value.to_str().ok());
    let freshness = conditional_get(&meta, if_none_match, if_modified_since);
    let mut response = Response::builder()
        .header(ETAG, tag)
        .header(LAST_MODIFIED, modified)
        .header(CACHE_CONTROL, "no-cache")
        .header(CONTENT_TYPE, content_type(&path))
        .header(CONTENT_LENGTH, meta.len.to_string());
    if matches!(freshness, Freshness::NotModified) {
        response = response.status(StatusCode::NOT_MODIFIED);
        return finish(response, Body::empty());
    }
    response = response.status(StatusCode::OK);
    if request.method() == Method::HEAD {
        return finish(response, Body::empty());
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => finish(response, Body::from(bytes)),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn finish(response: axum::http::response::Builder, body: Body) -> Response {
    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "reg-admin-{}-{}",
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

    fn meta_of(path: &Path) -> FileMeta {
        file_meta(path).unwrap()
    }

    mod conditional {
        use super::*;

        fn sample() -> (TempDir, PathBuf, FileMeta) {
            let dir = TempDir::new();
            let path = dir.0.join("note.txt");
            std::fs::write(&path, b"abcd").unwrap();
            let meta = meta_of(&path);
            (dir, path, meta)
        }

        #[test]
        fn etag_is_mtime_and_length() {
            let (_dir, _path, meta) = sample();
            assert_eq!(meta.len, 4);
            assert_eq!(
                etag(meta.mtime_secs, meta.len),
                format!("\"{}-4\"", meta.mtime_secs)
            );
        }

        #[test]
        fn if_none_match_is_304() {
            let (_dir, _path, meta) = sample();
            let tag = etag(meta.mtime_secs, meta.len);
            assert!(matches!(
                conditional_get(&meta, Some(&tag), None),
                Freshness::NotModified
            ));
            assert!(matches!(
                conditional_get(&meta, Some("*"), None),
                Freshness::NotModified
            ));
        }

        #[test]
        fn other_tag_is_200() {
            let (_dir, _path, meta) = sample();
            let since = last_modified(meta.mtime_secs);
            assert!(matches!(
                conditional_get(&meta, Some("\"other\""), Some(&since)),
                Freshness::Full
            ));
        }

        #[test]
        fn if_modified_since_is_304() {
            let (_dir, _path, meta) = sample();
            let since = last_modified(meta.mtime_secs);
            assert!(matches!(
                conditional_get(&meta, None, Some(&since)),
                Freshness::NotModified
            ));
            let earlier = last_modified(meta.mtime_secs.saturating_sub(1));
            assert!(matches!(
                conditional_get(&meta, None, Some(&earlier)),
                Freshness::Full
            ));
        }

        #[test]
        fn changed_file_is_200() {
            let (_dir, path, meta) = sample();
            let old = etag(meta.mtime_secs, meta.len);
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            use std::io::Write;
            file.write_all(b"wxyz").unwrap();
            let next = meta.mtime_secs + 10;
            file.set_modified(UNIX_EPOCH + Duration::from_secs(next))
                .unwrap();
            drop(file);
            let updated = meta_of(&path);
            let new_tag = etag(updated.mtime_secs, updated.len);
            assert_ne!(new_tag, old);
            assert!(matches!(
                conditional_get(&updated, Some(&old), None),
                Freshness::Full
            ));
        }
    }

    mod public_path {
        use super::*;

        fn tree() -> (TempDir, PathBuf) {
            let parent = TempDir::new();
            let root = parent.0.join("public");
            std::fs::create_dir_all(root.join("assets")).unwrap();
            std::fs::write(root.join("assets/app.js"), b"console.log(1)").unwrap();
            std::fs::write(parent.0.join("secret.txt"), b"secret").unwrap();
            (parent, root)
        }

        #[test]
        fn asset_resolves() {
            let (_parent, root) = tree();
            let file = safe_public_file(&root, "/assets/app.js").unwrap();
            assert_eq!(file, root.join("assets/app.js").canonicalize().unwrap());
        }

        #[test]
        fn parent_segment_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets/../secret.txt").is_none());
            assert!(safe_public_file(&root, "/assets/%2e%2e/secret.txt").is_none());
        }

        #[test]
        fn backslash_and_root_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets\\app.js").is_none());
            assert!(safe_public_file(&root, "/").is_none());
        }

        #[test]
        fn missing_file_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets/missing.js").is_none());
        }

        #[test]
        fn directory_refused() {
            let (_parent, root) = tree();
            assert!(safe_public_file(&root, "/assets").is_none());
        }
    }

    mod static_route {
        use super::*;
        use axum::body::to_bytes;
        use axum::http::Request;
        use tower::ServiceExt;

        fn tree() -> (TempDir, PathBuf) {
            let parent = TempDir::new();
            let root = parent.0.join("public");
            std::fs::create_dir_all(root.join("assets")).unwrap();
            std::fs::write(root.join("assets/app.js"), b"console.log(1)").unwrap();
            std::fs::write(parent.0.join("secret.txt"), b"secret").unwrap();
            (parent, root)
        }

        async fn call(root: &Path, request: Request<Body>) -> Response {
            admin_router(root.to_path_buf())
                .oneshot(request)
                .await
                .unwrap()
        }

        fn get(path: &str) -> Request<Body> {
            Request::builder().uri(path).body(Body::empty()).unwrap()
        }

        #[tokio::test]
        async fn static_get_200() {
            let (_parent, root) = tree();
            let response = call(&root, get("/assets/app.js")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "text/javascript"
            );
            let expected = std::fs::read(root.join("assets/app.js")).unwrap();
            assert_eq!(
                response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                expected.len().to_string()
            );
            assert!(response.headers().get(ETAG).is_some());
            assert!(response.headers().get(LAST_MODIFIED).is_some());
            assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-cache");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(body.as_ref(), expected.as_slice());
        }

        #[tokio::test]
        async fn static_revalidate_304() {
            let (_parent, root) = tree();
            let first = call(&root, get("/assets/app.js")).await;
            let tag = first.headers().get(ETAG).unwrap().to_str().unwrap().to_string();
            let request = Request::builder()
                .uri("/assets/app.js")
                .header(IF_NONE_MATCH, tag)
                .body(Body::empty())
                .unwrap();
            let response = call(&root, request).await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert!(response.headers().get(ETAG).is_some());
            assert!(response.headers().get(LAST_MODIFIED).is_some());
            assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-cache");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(body.is_empty());
        }

        #[tokio::test]
        async fn static_head() {
            let (_parent, root) = tree();
            let len = std::fs::metadata(root.join("assets/app.js")).unwrap().len();
            let request = Request::builder()
                .method(Method::HEAD)
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap();
            let response = call(&root, request).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .unwrap()
                    .to_str()
                    .unwrap(),
                len.to_string()
            );
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(body.is_empty());
        }

        #[tokio::test]
        async fn static_escape_404() {
            let (_parent, root) = tree();
            let response = call(&root, get("/assets/../secret.txt")).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(response.headers().get(ETAG).is_none());
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(body.is_empty());
        }

        #[tokio::test]
        async fn static_post_405() {
            let (_parent, root) = tree();
            let request = Request::builder()
                .method(Method::POST)
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap();
            let response = call(&root, request).await;
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        }
    }
}
