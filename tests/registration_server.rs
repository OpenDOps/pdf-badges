use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use rust_reg::registration_server::{serve, App};

#[tokio::test]
async fn http_and_websocket_share_the_server() {
    let (app, worker) = App::new();
    app.store
        .write_batch(vec![rust_reg::registration_server::Registration {
            id: "1".into(),
            name: "Ann".into(),
        }]);

    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_addr = http.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let server = tokio::spawn(serve(http, app, worker, shutdown_rx));

    let health = reqwest::get(format!("http://{http_addr}/health"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(health, "{\"status\":\"ok\"}");

    let missing = reqwest::get(format!("http://{http_addr}/registrations/missing"))
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

    let found = reqwest::get(format!("http://{http_addr}/registrations/1"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(found, "{\"id\":\"1\",\"name\":\"Ann\"}");

    let (mut socket, _) = connect_async(format!("ws://{http_addr}/ws")).await.unwrap();
    socket.send(Message::Text("1".into())).await.unwrap();
    let reply = socket.next().await.unwrap().unwrap();
    assert_eq!(
        reply,
        Message::Text("{\"id\":\"1\",\"name\":\"Ann\"}".into())
    );
    socket.send(Message::Text("missing".into())).await.unwrap();
    let reply = socket.next().await.unwrap().unwrap();
    assert_eq!(reply, Message::Text("{\"found\":false}".into()));

    let queued = reqwest::Client::new()
        .post(format!("http://{http_addr}/print"))
        .json(&serde_json::json!({"id": "badge", "pages": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(queued.status(), reqwest::StatusCode::ACCEPTED);

    shutdown_tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
