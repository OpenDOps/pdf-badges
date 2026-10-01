use std::time::Duration;

use futures::StreamExt;
use tokio::sync::watch;

use crate::registration_server::gate::Gate;
use crate::registration_server::store::{Registration, Store};
use crate::registration_server::App;

/// Two sockets in flight. Parsing happens one body at a time after they return.
pub const SYNC_IN_FLIGHT: usize = 2;
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
enum DownloadError {
    Http(reqwest::Error),
    Status(reqwest::StatusCode),
    TooLarge,
    Parse(serde_json::Error),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Http(err) => write!(f, "{err}"),
            DownloadError::Status(status) => write!(f, "remote status {status}"),
            DownloadError::TooLarge => write!(f, "body exceeds {MAX_BODY_BYTES} bytes"),
            DownloadError::Parse(err) => write!(f, "{err}"),
        }
    }
}

impl From<reqwest::Error> for DownloadError {
    fn from(err: reqwest::Error) -> Self {
        DownloadError::Http(err)
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(SYNC_IN_FLIGHT)
        .build()
        .expect("registration sync client")
}

pub async fn sync_loop(
    app: App,
    endpoints: Vec<String>,
    interval: Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    let client = client();
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                sync_once(&client, &endpoints, &app.store, &app.gate).await;
            }
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

pub async fn sync_once(client: &reqwest::Client, endpoints: &[String], store: &Store, gate: &Gate) {
    futures::stream::iter(endpoints.iter().cloned())
        .map(|url| {
            let client = client.clone();
            async move {
                gate.wait_until_idle().await;
                let body = read_body(&client, &url).await;
                (url, body)
            }
        })
        .buffer_unordered(SYNC_IN_FLIGHT)
        .for_each(|(url, body)| async move {
            gate.wait_until_idle().await;
            match body {
                Ok(bytes) => match parse_batch(&bytes) {
                    Ok(rows) => store.write_batch(rows),
                    Err(err) => eprintln!("registration-server sync {url}: {err}"),
                },
                Err(err) => eprintln!("registration-server sync {url}: {err}"),
            }
        })
        .await;
}

async fn read_body(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, DownloadError> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(DownloadError::Status(response.status()));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(DownloadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_batch(bytes: &[u8]) -> Result<Vec<Registration>, DownloadError> {
    serde_json::from_slice(bytes).map_err(DownloadError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve_json(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
        format!("http://{addr}/rows")
    }

    #[tokio::test]
    async fn two_endpoints_land_as_one_batch_each() {
        let first = serve_json(r#"[{"id":"1","name":"Ann"}]"#).await;
        let second = serve_json(r#"[{"id":"2","name":"Bo"}]"#).await;
        let store = Store::new();
        let gate = Gate::new();
        sync_once(&client(), &[first, second], &store, &gate).await;
        assert_eq!(store.lookup("1").unwrap().name, "Ann");
        assert_eq!(store.lookup("2").unwrap().name, "Bo");
    }

    #[tokio::test]
    async fn a_local_call_holds_the_download() {
        let url = serve_json(r#"[{"id":"1","name":"Ann"}]"#).await;
        let store = Arc::new(Store::new());
        let gate = Arc::new(Gate::new());
        let guard = gate.enter_request();
        let pending = {
            let store = Arc::clone(&store);
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { sync_once(&client(), &[url], &store, &gate).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(store.lookup("1").is_none());
        drop(guard);
        pending.await.unwrap();
        assert_eq!(store.lookup("1").unwrap().name, "Ann");
    }
}
