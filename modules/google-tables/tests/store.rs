use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use google_tables::irbis::google_tables::v1::{ChangeKind, SnapshotReason};
use google_tables::model::CellValue;
use google_tables::store::{
    map_headroom, CommitOptions, MapHeadroom, ScannedCell, ScannedRow, Store,
};

fn scratch() -> (PathBuf, Store) {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join(format!("google-tables-{nanos}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("google-tables.lmdb");
    let store = Store::open(&path).unwrap();
    (dir, store)
}

fn text_cell(name: &str, excel_id: &str, text: &str) -> ScannedCell {
    ScannedCell {
        name: name.into(),
        excel_id: excel_id.into(),
        value: CellValue::Text(text.into()),
        exclude_from_hash: false,
    }
}

fn row(uid: &str, sheet_row: u32, fingerprint: &str, cells: Vec<ScannedCell>) -> ScannedRow {
    ScannedRow {
        uid: uid.into(),
        sheet_row,
        fingerprint: fingerprint.into(),
        cells,
        cells_known: true,
    }
}

fn kind(value: i32) -> ChangeKind {
    ChangeKind::try_from(value).unwrap()
}

#[test]
fn store_add_update_delete() {
    let (_dir, store) = scratch();
    let wid = 1;
    let original = row(
        "UID0000000000001",
        5,
        "fp-oslo",
        vec![
            text_cell("Name", "A", "Ann"),
            text_cell("City", "B", "Oslo"),
        ],
    );
    store
        .commit(
            wid,
            &[original.clone()],
            CommitOptions {
                version: Some("v1".into()),
                ..CommitOptions::default()
            },
        )
        .unwrap();
    let events = store.changes(wid).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(kind(events[0].kind), ChangeKind::Added);
    assert_eq!(events[0].uid, "UID0000000000001");

    store
        .commit(wid, &[original.clone()], CommitOptions::default())
        .unwrap();
    assert_eq!(store.changes(wid).unwrap().len(), 1);
    assert_eq!(store.watch(wid).unwrap().drive_version, "v1");

    let moved = row(
        "UID0000000000001",
        5,
        "fp-riga",
        vec![
            text_cell("Name", "A", "Ann"),
            text_cell("City", "B", "Riga"),
        ],
    );
    store
        .commit(wid, &[moved], CommitOptions::default())
        .unwrap();
    let events = store.changes(wid).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(kind(events[1].kind), ChangeKind::Updated);
    assert_eq!(events[1].changed_columns, vec!["City".to_string()]);

    store.commit(wid, &[], CommitOptions::default()).unwrap();
    let events = store.changes(wid).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(kind(events[2].kind), ChangeKind::Deleted);
    assert_eq!(events[2].uid, "UID0000000000001");
    assert!(store.read_stored(wid).unwrap().rows.is_empty());
}

#[test]
fn store_clear_reports_the_column() {
    let (_dir, store) = scratch();
    let wid = 5;
    store
        .commit(
            wid,
            &[row(
                "UID0000000000005",
                4,
                "fp-oslo",
                vec![
                    text_cell("Name", "A", "Ann"),
                    text_cell("City", "B", "Oslo"),
                ],
            )],
            CommitOptions::default(),
        )
        .unwrap();
    store
        .commit(
            wid,
            &[row(
                "UID0000000000005",
                4,
                "fp-cleared",
                vec![text_cell("Name", "A", "Ann")],
            )],
            CommitOptions::default(),
        )
        .unwrap();
    let events = store.changes(wid).unwrap();
    assert_eq!(kind(events[1].kind), ChangeKind::Updated);
    assert_eq!(events[1].changed_columns, vec!["City".to_string()]);
}

#[test]
fn store_move_is_quiet() {
    let (_dir, store) = scratch();
    let wid = 2;
    let at_five = row(
        "UID0000000000002",
        5,
        "fp-ann",
        vec![
            text_cell("Name", "A", "Ann"),
            text_cell("City", "B", "Oslo"),
        ],
    );
    store
        .commit(wid, &[at_five.clone()], CommitOptions::default())
        .unwrap();
    let before = store.index(wid).unwrap();
    assert_eq!(before[0].sheet_row, 5);

    let mut at_nine = at_five;
    at_nine.sheet_row = 9;
    store
        .commit(wid, &[at_nine], CommitOptions::default())
        .unwrap();
    assert_eq!(store.changes(wid).unwrap().len(), 1);
    let after = store.index(wid).unwrap();
    assert_eq!(after[0].sheet_row, 9);
    assert_eq!(after[0].content_hash, before[0].content_hash);
    let snapshot = store.read_stored(wid).unwrap();
    assert_eq!(snapshot.rows[0].sheet_row, 9);
}

#[test]
fn store_mass_delete_snapshots() {
    let (_dir, store) = scratch();
    let wid = 3;
    let stored: Vec<ScannedRow> = (0..10)
        .map(|index| {
            row(
                &format!("U{index:015}"),
                index + 2,
                &format!("fp-{index}"),
                vec![text_cell("Name", "A", &format!("n{index}"))],
            )
        })
        .collect();
    store
        .commit(
            wid,
            &stored,
            CommitOptions {
                version: Some("base".into()),
                snapshot: true,
                ..CommitOptions::default()
            },
        )
        .unwrap();

    let kept = stored[..6].to_vec();
    let applied = store
        .commit(
            wid,
            &kept,
            CommitOptions {
                version: Some("boom".into()),
                ..CommitOptions::default()
            },
        )
        .unwrap();
    assert_eq!(applied.status, "running");
    let deleted: Vec<_> = store
        .changes(wid)
        .unwrap()
        .into_iter()
        .filter(|event| kind(event.kind) == ChangeKind::Deleted)
        .map(|event| event.uid)
        .collect();
    assert_eq!(
        deleted,
        vec![
            "U000000000000006".to_string(),
            "U000000000000007".to_string(),
            "U000000000000008".to_string(),
            "U000000000000009".to_string(),
        ]
    );
    assert_eq!(store.read_stored(wid).unwrap().rows.len(), 6);
    assert_eq!(store.watch(wid).unwrap().drive_version, "boom");
    let snapshots = store.list_snapshots(wid).unwrap();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].reason, SnapshotReason::Initial as i32);
    assert_eq!(snapshots[0].rows.len(), 10);
    assert_eq!(snapshots[1].reason, SnapshotReason::MassiveDelete as i32);
    assert_eq!(snapshots[1].rows.len(), 10);
    assert!(snapshots[0].created_at_unix_ms <= snapshots[1].created_at_unix_ms);
    let kept_uids: Vec<_> = snapshots[1]
        .rows
        .iter()
        .map(|row| row.uid.clone())
        .filter(|uid| kept.iter().any(|row| row.uid == *uid))
        .collect();
    assert_eq!(kept_uids.len(), 6);
    assert!(store.get_snapshot(wid, 99).unwrap().is_none());
}

#[test]
fn store_snapshot_keeps_ten() {
    let (_dir, store) = scratch();
    let wid = 6;
    let rows: Vec<ScannedRow> = (0..10)
        .map(|index| {
            row(
                &format!("U{index:015}"),
                index + 2,
                &format!("fp-{index}"),
                vec![text_cell("Name", "A", &format!("n{index}"))],
            )
        })
        .collect();
    store
        .commit(
            wid,
            &rows,
            CommitOptions {
                version: Some("base".into()),
                snapshot: true,
                ..CommitOptions::default()
            },
        )
        .unwrap();
    let other = 7;
    store
        .commit(
            other,
            &rows[..2],
            CommitOptions {
                version: Some("other".into()),
                snapshot: true,
                ..CommitOptions::default()
            },
        )
        .unwrap();

    let kept = rows[..6].to_vec();
    for _ in 0..10 {
        store
            .commit(
                wid,
                &kept,
                CommitOptions {
                    version: Some("cut".into()),
                    ..CommitOptions::default()
                },
            )
            .unwrap();
        store
            .commit(
                wid,
                &rows,
                CommitOptions {
                    version: Some("back".into()),
                    ..CommitOptions::default()
                },
            )
            .unwrap();
    }

    let snapshots = store.list_snapshots(wid).unwrap();
    assert_eq!(snapshots.len(), 10);
    assert_eq!(snapshots[0].snapshot_id, 2);
    assert_eq!(snapshots.last().unwrap().snapshot_id, 11);
    assert!(snapshots
        .windows(2)
        .all(|pair| pair[0].created_at_unix_ms <= pair[1].created_at_unix_ms));
    assert!(store.get_snapshot(wid, 1).unwrap().is_none());
    assert_eq!(store.get_snapshot(wid, 11).unwrap().unwrap().rows.len(), 10);
    assert_eq!(store.list_snapshots(other).unwrap().len(), 1);
}

#[test]
fn store_map_rejects_a_larger_file() {
    let (dir, store) = scratch();
    drop(store);
    let path = dir.join("too-big.lmdb");
    let file = std::fs::File::create(&path).unwrap();
    let map = 1024 * 1024;
    file.set_len(map as u64 + 1).unwrap();
    match Store::open_with_map_size(&path, map) {
        Err(err) => {
            let message = err.to_string();
            assert!(message.contains("larger"), "{message}");
        }
        Ok(_) => panic!("a file larger than the map opened"),
    }
}

#[test]
fn store_map_headroom_warns() {
    assert_eq!(map_headroom(0, 100).unwrap(), MapHeadroom::Plenty);
    assert_eq!(map_headroom(89, 100).unwrap(), MapHeadroom::Plenty);
    assert_eq!(
        map_headroom(90, 100).unwrap(),
        MapHeadroom::Low { free_percent: 10 }
    );
    assert_eq!(
        map_headroom(94, 100).unwrap(),
        MapHeadroom::Low { free_percent: 6 }
    );
    assert_eq!(
        map_headroom(95, 100).unwrap(),
        MapHeadroom::Critical { free_percent: 5 }
    );
    assert_eq!(
        map_headroom(100, 100).unwrap(),
        MapHeadroom::Critical { free_percent: 0 }
    );
    assert!(map_headroom(101, 100).is_err());
}

#[test]
fn store_snapshot_seq() {
    let (_dir, store) = scratch();
    let wid = 4;
    let rows = vec![
        row(
            "UID000000000000A",
            2,
            "a",
            vec![text_cell("Name", "A", "Ann")],
        ),
        row(
            "UID000000000000B",
            3,
            "b",
            vec![text_cell("Name", "A", "Bo")],
        ),
    ];
    store.commit(wid, &rows, CommitOptions::default()).unwrap();
    let snapshot = store.read_stored(wid).unwrap();
    let events = store.changes(wid).unwrap();
    assert_eq!(snapshot.snapshot_seq, events.last().unwrap().seq);
    assert_eq!(snapshot.rows.len(), 2);
    assert_eq!(snapshot.snapshot_seq, 2);
}
