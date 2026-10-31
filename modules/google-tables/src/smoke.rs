//! In-process run of the step 8 fixture. Prints `SNAPSHOT`, then `UPDATED`.

use std::time::Duration;

use google_tables::google::FakeBook;
use google_tables::irbis::google_tables::v1::{
    ChangeKind, EnsureIdentityRequest, OpenSpreadsheetRequest, SelectHeaderRequest,
    StartWatchRequest,
};
use google_tables::server::GoogleTablesService;

const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/spreadsheet.json");

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = std::env::temp_dir().join(format!("google-tables-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let service = GoogleTablesService::open_manual(dir.join("google-tables.lmdb"))?;
    service.insert_book(FakeBook::from_fixture("sheet-1", FIXTURE)?);

    let view = service.open_spreadsheet(OpenSpreadsheetRequest {
        spreadsheet_id: "sheet-1".into(),
        credentials_ref: "local".into(),
        header_candidate_limit: 0,
    })?;
    service.select_header(SelectHeaderRequest {
        session_id: view.session_id.clone(),
        excel_row: 2,
        subcolumn_row: 0,
    })?;
    service.ensure_identity(EnsureIdentityRequest {
        session_id: view.session_id.clone(),
    })?;
    let watch = service.start_watch(StartWatchRequest {
        session_id: view.session_id,
        drive_push: false,
    })?;
    settle(&service).await;
    let snapshot = service.changes(&watch.watch_id)?;
    for event in &snapshot {
        println!("{}", kind_name(event.kind));
    }

    service
        .with_book("sheet-1", |book| {
            book.tab_mut(0).expect("sheet").put_text(4, 1, "Bergen");
            book.version += 1;
        })
        .expect("book");
    settle(&service).await;
    let events = service.changes(&watch.watch_id)?;
    for event in events.iter().skip(snapshot.len()) {
        println!("{}", kind_name(event.kind));
    }
    service.shutdown().await;

    let names: Vec<_> = events.iter().map(|event| kind_name(event.kind)).collect();
    if names == ["SNAPSHOT", "UPDATED"] {
        Ok(())
    } else {
        Err(format!("queued kinds {names:?}").into())
    }
}

async fn settle(service: &GoogleTablesService) {
    tokio::time::timeout(Duration::from_secs(30), service.quiet_blocked())
        .await
        .expect("watch did not reach the quiet wait");
    service.advance_quiet().await;
    tokio::time::timeout(Duration::from_secs(30), service.quiet_blocked())
        .await
        .expect("watch did not reach the quiet wait");
    service.advance_quiet().await;
}

fn kind_name(kind: i32) -> &'static str {
    match ChangeKind::try_from(kind) {
        Ok(ChangeKind::Added) => "ADDED",
        Ok(ChangeKind::Updated) => "UPDATED",
        Ok(ChangeKind::Deleted) => "DELETED",
        Ok(ChangeKind::Mapping) => "MAPPING",
        Ok(ChangeKind::Snapshot) => "SNAPSHOT",
        _ => "UNSPECIFIED",
    }
}
