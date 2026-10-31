use google_tables::irbis::google_tables::v1::{ChangeKind, RowChange};
use google_tables::irbis::table::v1::{value, Cell, Row, Value};
use google_tables::server::{serve_listener, GoogleTablesService, DEFAULT_ADDR};
use prost::Message;
use tokio::net::TcpListener;
use tonic::transport::Endpoint;
use tonic::Code;

#[test]
fn proto_roundtrip() {
    let change = RowChange {
        seq: 1,
        kind: ChangeKind::Added as i32,
        uid: "0123456789ABCDEF".into(),
        row: Some(Row {
            excel_row: 4,
            cells: vec![Cell {
                excel_id: "A".into(),
                value: Some(Value {
                    kind: Some(value::Kind::Text("Ann".into())),
                }),
            }],
        }),
        content_hash: Vec::new(),
        changed_columns: Vec::new(),
        columns: Vec::new(),
    };
    let parsed = RowChange::decode(change.encode_to_vec().as_slice()).unwrap();
    assert_eq!(parsed, change);
    assert_eq!(parsed.kind, ChangeKind::Added as i32);
}

#[tokio::test]
async fn proto_unknown_watch() {
    assert!(DEFAULT_ADDR.ends_with(":50052"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve_listener(listener).await.unwrap();
    });
    let endpoint = Endpoint::from_shared(format!("http://{addr}")).unwrap();
    let mut client =
        google_tables::irbis::google_tables::v1::google_tables_client::GoogleTablesClient::connect(
            endpoint,
        )
        .await
        .unwrap();
    let status = client
        .get_watch(google_tables::irbis::google_tables::v1::GetWatchRequest {
            watch_id: "missing".into(),
        })
        .await
        .expect_err("unknown watch");
    assert_eq!(status.code(), Code::NotFound);
    let _ = GoogleTablesService::new();
}
