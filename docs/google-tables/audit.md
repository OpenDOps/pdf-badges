# google-tables implementation audit

Scope: `modules/google-tables` as of steps 1–8 of [implementation-plan.md](implementation-plan.md), checked against [design.md](design.md). Focus: sync, content hashes, UID repair, column metadata, plus logic and performance problems that will surface at step 10 (live sheet) or at scale.

Baseline: `cargo test -p google-tables` is green (cycle 17, grid 2, grid_payload 3, proto 2, session 7, store 4, uid 3, watch 12). The issues below are not caught by those tests.

Severity:

- **High**: wrong data, lost events, a stuck watch, or a failure that is certain once the real Google client or a large sheet is used.
- **Medium**: a design rule that is not met, or a defect with a narrow trigger.
- **Low**: inefficiency, unclear reporting, or documentation drift.

## Summary

| ID | Severity | Area | Issue |
|---|---|---|---|
| ✅ [GT-01](#gt-01-rows-below-sheet-row-10000-are-deleted) | High | Sync | Rows below sheet row 10,000 are invisible and get deleted |
| ✅ [GT-02](#gt-02-subscribe-cannot-replay-more-than-1000-events) | High | Queue | `Subscribe` cannot replay more than 1,000 events, so a resume never catches up |
| ✅ [GT-03](#gt-03-the-cycle-blocks-the-async-runtime-behind-one-global-mutex) | High | Sync / perf | The cycle blocks the async runtime behind one global mutex |
| ✅ [GT-04](#gt-04-a-held-mass-delete-rescans-every-10-seconds) | High | Sync / quota | A held mass delete rescans every 10 seconds |
| ✅ [GT-05](#gt-05-one-range-per-row-in-fetches-and-uid-writes) | High | Perf / API | One range per row in fetches and UID writes |
| ✅ [GT-06](#gt-06-an-edited-uid-is-not-written-back) | High | UID | An edited UID is not written back, so plan step 10 fails |
| ✅ [GT-07](#gt-07-no-shared-rate-limiter) | High | Quota | No shared rate limiter, in code or in the plan |
| ✅ [GT-08](#gt-08-confirm-is-accepted-without-needs_confirm-and-stays-armed) | Medium | Sync | `confirm` is accepted without `needs_confirm` and stays armed |
| ✅ [GT-09](#gt-09-snapshot-replaces-added-whenever-the-index-is-empty) | Medium | Events | `SNAPSHOT` replaces `ADDED` whenever the index is empty |
| ✅ [GT-10](#gt-10-changed_columns-omits-cleared-cells) | Medium | Hash / events | `changed_columns` omits cleared cells |
| [GT-11](#gt-11-deleted-rows-carry-anonymous-cells) | Medium | Events | `DELETED` rows carry anonymous cells |
| [GT-12](#gt-12-mapping-is-not-implemented) | Medium | Columns | `MAPPING` is not implemented. A re-watch with new columns floods `UPDATED` |
| [GT-13](#gt-13-stopwatch-does-not-survive-a-restart) | Medium | Watch | `StopWatch` does not survive a restart |
| [GT-14](#gt-14-cycle-errors-are-not-recorded-or-backed-off) | Medium | Watch | Cycle errors are not recorded or backed off |
| [GT-15](#gt-15-service-column-check-is-a-call-on-every-cycle) | Medium | Columns / perf | Service-column check is a call on every cycle |
| [GT-16](#gt-16-uid-writes-are-addressed-by-row-number) | Medium | UID | UID writes are addressed by row number, which can race a row insert |
| [GT-17](#gt-17-fingerprint-collisions-hide-real-edits) | Medium | Hash | Fingerprint collisions hide real edits |
| [GT-18](#gt-18-excluding-every-mapped-column-empties-the-watch) | Medium | Columns | Excluding every mapped column empties the watch |
| ✅ [GT-19](#gt-19-formula-comparison-is-exact-text) | Medium | Columns | Formula comparison is exact text. Real Sheets can cause a rewrite loop |
| ✅ [GT-20](#gt-20-quadratic-lookups-in-the-cycle-and-commit) | Medium | Perf | Quadratic lookups in the cycle and the commit |
| ✅ [GT-21](#gt-21-event-log-reads-scan-the-whole-prefix) | Medium | Perf | Event-log reads scan the whole prefix. One `Notify` wakes every subscriber |
| [GT-22](#gt-22-subscriber-tasks-leak-after-disconnect) | Medium | Queue | Subscriber tasks leak after disconnect |
| ✅ [GT-23](#gt-23-lmdb-map-size-is-fixed-at-1-gib) | Medium | Store | LMDB map size is fixed at 1 GiB |
| [GT-24](#gt-24-a-watch-without-a-header-loses-row-1) | Medium | Columns | A watch without a header loses row 1 |
| [GT-25](#gt-25-column_missing-moves-the-baseline-version) | Low | Sync | `column_missing` moves the baseline version |
| [GT-26](#gt-26-five-version-reads-per-change) | Low | Perf | Five `version` reads per change |
| [GT-27](#gt-27-a-column-move-refetches-every-row) | Low | Hash / perf | A column move refetches every row |
| [GT-28](#gt-28-new-rows-are-downloaded-twice-when-a-uid-is-missing) | Low | UID / perf | New rows are downloaded twice when a UID is missing |
| [GT-29](#gt-29-uid-decisions-run-on-a-stale-formula) | Low | UID | UID decisions run on a stale formula |
| [GT-30](#gt-30-excluded-cells-are-stored-and-go-stale) | Low | Hash | Excluded cells are stored and go stale |
| [GT-31](#gt-31-syncreport-hides-the-outcome) | Low | Sync | `SyncReport` hides the outcome |
| [GT-32](#gt-32-no-jitter-and-a-restart-herd) | Low | Quota | No jitter, and a restart herd |
| [GT-33](#gt-33-startwatch-keeps-an-old-status) | Low | Watch | `StartWatch` keeps an old status |
| [GT-34](#gt-34-fakebook-differs-from-sheets-in-two-places) | Low | Tests | `FakeBook` differs from Sheets in two places |
| [GT-35](#gt-35-documentation-drift) | Low | Docs | Documentation drift |

## High

### GT-01. Rows below sheet row 10,000 are deleted

✅ **Status: fixed.** The two-column read is an open range (`end_row: None`). `cycle_reads_past_10000` keeps sheet row 10,001.

**Where:** `cycle.rs`, `COLUMN_END = 10_000`, the two-column `batch_get` in `run_cycle`.

**Flow:** Every cycle reads the UID column and the fingerprint column from the first data row to row 10,000. `read_service_rows` builds the scan from that window. `Store::commit` treats a stored UID that is missing from the scan as deleted.

**What is broken:** The design says the read runs "through the last non-empty cell of either column". A sheet with more than 10,000 rows loses every row past 10,000. If those rows are 30 percent or less of the stored rows, the commit appends `DELETED` for each of them with no hold. The rows come back as `ADDED` only if the sheet shrinks.

**Fix:** Use open-ended ranges (`D3:D`, `E1:E`) in the real client, and model the fake range as `end_row: None`. If a hard cap is kept for memory, set the status to an error such as `too_many_rows` and stop the cycle. Do not commit a truncated scan. Add `cycle_reads_past_10000`.

### GT-02. `Subscribe` cannot replay more than 1,000 events

✅ **Status: fixed.** Replay uses `send().await`. A live gap above 1,000 closes the stream with `resource_exhausted`, sent with a one-second timeout. `watch_resume_large_backlog` replays 2,500 events.

**Where:** `watch.rs`, `subscribe`. The producer loop uses `tx.capacity() == 0` and `tx.try_send`.

**Flow:** The task reads every event after the cursor and pushes them with `try_send` in a tight loop. When the channel is full, it tries to send `resource_exhausted` and returns.

**What is broken:**

1. The loop never yields, so the gRPC side cannot drain it. Any backlog over 1,000 events fills the channel at once. A bulk edit of 2,000 rows, or a resume after downtime, does this.
2. The `resource_exhausted` error is sent with `try_send` into a channel that is already full, so it is dropped. The stream just ends.
3. The client resumes from its last cursor, hits the same backlog, and fails again. It never catches up.

The design's "buffers 1,000 events, then fails" is meant for a live subscriber that stops reading. It is not meant for replay.

**Fix:** Replay with `tx.send(event).await` so replay has backpressure. Apply the 1,000 limit only to the live path, as lag: if `last_seq - cursor > 1000` while following, fail the stream. Send the terminal error with `send().await` and a timeout so it arrives. Add `watch_resume_large_backlog` with 2,500 events.

### GT-03. The cycle blocks the async runtime behind one global mutex

✅ **Status: fixed.** `GoogleSheets` is async. Each watch has its own mutex, and a 429 waits on the quiet clock. `watch_cycle_backoff` rate-limits `batch_get` on the default test runtime.

**Where:** `watch.rs`: `GoogleTablesService::run_cycle`, `wait_quiet`, `watch_loop`, `sync_now`. `session.rs`: `Inner.cycle: Mutex<()>`.

**Flow:** `watch_loop` is a tokio task that calls the synchronous `run_cycle`. `sync_now` is called from the async tonic handler and runs the cycle inline. `run_cycle` takes `inner.cycle`, a process-wide `std::sync::Mutex`, and holds it through LMDB writes and every Google call. On a 429, the retry sleep runs `wait_quiet`, which calls `block_in_place(|| handle.block_on(quiet.wait(delay)))`.

**What is broken:**

- Only one cycle runs in the whole process. A 429 backoff of up to 60 seconds inside one watch stalls every other watch and every `SyncNow`, while they block runtime worker threads on the std mutex.
- `block_in_place` panics on a current-thread runtime. `#[tokio::test]` uses that runtime by default, so any watch test that hits a 429 inside the cycle will panic. Today's tests only rate-limit the version poll, so this path is untested.
- With the real HTTPS client in step 10, each Sheets call blocks a runtime thread. A few slow calls starve tonic.

**Fix:** Make `GoogleSheets` async, or run each cycle in `tokio::task::spawn_blocking` and give it a sleep closure that uses `std::thread::sleep`, or an injected clock in tests. Replace the global mutex with a per-`wid` mutex, for example `HashMap<u32, Arc<tokio::sync::Mutex<()>>>`. The limiter in GT-07 should be the only cross-watch serialization. Add a test that rate-limits a `batch_get` inside a watch cycle on the default test runtime.

### GT-04. A held mass delete rescans every 10 seconds

✅ **Status: fixed.** A mass delete no longer holds. The commit stores a `massive_delete` snapshot and applies the deletes, then the baseline advances, so an unchanged file polls every 60 seconds.

**Where:** `watch.rs`, `watch_loop`. `store.rs`, `commit`: on hold, the version is not stored.

**Flow:** The commit holds, so `drive_version` keeps the old baseline. On the next loop iteration, `live != record.drive_version` is still true, so the loop waits 10 seconds and runs a full cycle: version, metadata, two-column read, mapped fetch of changed rows, version, then hold again. This repeats until someone calls `SyncNow(confirm)`.

**What is broken:** A watch that nobody confirms costs about 4–5 Sheets and Drive calls every 10 seconds, or roughly 25 reads per minute. The budget is 60 reads per minute per service account. Two held watches on one account will 429 every other watch. The design says the held state is "an ordinary rescan" on the next cycle, not a rescan every 10 seconds on an unchanged file.

**Fix:** On hold, store `held_version = V` on the record. In `watch_loop`, treat `live == held_version` as caught up and use the 60-second poll. A new `version` (a user fixing the sheet) or `SyncNow` runs a cycle. Clear `held_version` on commit. Add `watch_held_is_quiet`: after a hold, advancing the clock issues no `batch_get` until the version changes.

### GT-05. One range per row in fetches and UID writes

✅ **Status: fixed.** Consecutive rows share one range. More than half of the service rows are one block, and a request holds at most 100 ranges. UID writes are one value run per contiguous rows. `cycle_first_scan_is_one_range` reads rows 2–100 as one range.

**Where:** `cycle.rs`: `row_range`, the mapped fetch in `run_cycle`, the fetch in `decide_uids`, and the `write_raw` of `uid_writes`.

**Flow:** The first scan of a new watch marks every row changed. The mapped fetch builds one `ColRange` per row. `decide_uids` writes one cell per row.

**What is broken:** A 10,000-row sheet sends a `values.batchGet` with 10,000 `ranges` query parameters, about 200 KB of URL, which Google rejects. A `values.batchUpdate` with 10,000 single-cell ranges is valid but slow and large. Recreating the UID column (GT-15 flow) fetches every row the same way. The fake accepts all of this, so tests pass.

**Fix:** Coalesce consecutive rows into one range (`A120:C180`). Chunk at a fixed number of ranges per request, for example 100. When more than about half of the rows changed, read the whole mapped block in one range. Do the same for UID writes, one range per contiguous run with a value array. Add a cycle test that asserts the first scan of rows 2–100 issues one range.

### GT-06. An edited UID is not written back

✅ **Status: fixed.** An invalid UID or a valid foreign id is written back to the missing stored UID when the other cells still hash to it. `cycle_edited_uid_restored` covers both edits and emits no `DELETED` or `ADDED`.

**Where:** `cycle.rs`, `decide_uids`. Plan step 10, row "Edit a UID cell".

**Flow:** The plan expects this: an editor overwrites a UID, the sheet warns, the edit is kept, and "the next cycle writes the previous UID back when the other cells still match that hash". In the code:

- If the edited value is not a valid UID, `decide_uids` draws a new id. Restore by hash is tried only when `row.uid.is_empty()`.
- If the edited value is a valid id (pasted from another sheet, or typed in the alphabet), it is kept. The old UID is now missing from the column, so the next commit emits `DELETED` for the old UID and `ADDED` for the new one.

**What is broken:** Neither case restores the stored UID, so the step 10 acceptance row fails. Consumers see a delete and an add for a row nobody deleted.

**Fix:** In `decide_uids`, put rows with an invalid UID, and rows whose valid UID is neither stored nor duplicated, into the "restore" set when any stored UID is missing. Fetch their mapped cells. If the content hash matches a missing stored UID, write that UID back. Otherwise draw a new id for an invalid value and keep a valid foreign value. Record the rule in design.md step 4, which now says only "empty UID is filled with a stored UID". Add `cycle_edited_uid_restored` for both the invalid and the valid-foreign case.

### GT-07. No shared rate limiter

✅ **Status: fixed.** `Limiter` is keyed by `credentials_ref`, with read, write, and project buckets and one backoff exponent per account. The watch loop and setup writes go through it. `limiter_caps_reads` and `limiter_shares_backoff` cover the caps and the shared exponent.

**Where:** Nothing in `google.rs`, `cycle.rs`, or `watch.rs`. The plan has no step for it.

**Flow:** The design says: "Sheets and Drive calls share one limiter: 60 reads and 60 writes per minute per service account, 300 per minute per project. A 429 backs that account off". Today every watch loop and every cycle calls Google on its own. Backoff state (`failures`) is local to one `retry` call or one `live_version` call.

**What is broken:**

- With push off, every watch polls `version` every 60 seconds. 300 watches already reach the project limit before any change is read.
- A 429 seen by one watch does not pause the others on the same account. They keep calling and extend the throttling.
- `n` resets per call site, not "after the next success" on that account.

**Fix:** Add a `Limiter` keyed by `credentials_ref`, with two token buckets (reads and writes) plus one project bucket and the shared backoff exponent. Wrap the real client and the fake with it, and make the fake count tokens so tests can assert it. Add the limiter to plan step 10, or a new step between 9 and 10, with tests `limiter_shares_backoff` and `limiter_caps_reads`.

## Medium

### GT-08. `confirm` is accepted without `needs_confirm` and stays armed

✅ **Status: fixed.** `confirm` is gone. A mass delete stores a snapshot of the sheet and applies the deletes. `ListSnapshots` lists those copies by time, and `GetSnapshot` returns one.

**Where:** `watch.rs`, `run_cycle` and `Inner.confirm`. Test `watch_sync_confirm_follows_uid_write`.

**Flow:** `SyncNow(confirm: true)` puts the `wid` into an in-memory set. Every later cycle for that watch, including loop cycles, commits with `confirm = true` until one returns `Committed`.

**What is broken:**

- The design lists "`confirm` when the status is not `needs_confirm`" as an error. The code accepts it, and the new watch test sends `confirm` on a `running` watch to rely on it.
- The flag is a blank cheque. Suppose a user confirms 4 deletes, the cycle only writes UIDs, and then 60 percent of the sheet is deleted before the next stable read. That larger delete is applied without a new confirm.
- A restart loses the flag. A confirm sent just before a restart does nothing.

**Fix:** Return `FAILED_PRECONDITION` unless the status is `needs_confirm`. At hold time, persist the pending delete UIDs, or their hash and count, on the watch record. `confirm` approves exactly that set. A later commit applies the approved deletes and holds again if other deletes push it over the threshold. Change the test to reach `needs_confirm` first, then confirm across a UID write.

### GT-09. `SNAPSHOT` replaces `ADDED` whenever the index is empty

✅ **Status: fixed.** `SNAPSHOT` is used only when the log is still at sequence 0 and the index is empty. `cycle_refill_emits_added` clears a committed sheet and pastes new rows, and those rows are `ADDED`.

**Where:** `cycle.rs`, `let snapshot = stored.is_empty();`.

**Flow:** The design says the first cycle of a new watch appends one `SNAPSHOT`. The code sets `snapshot` whenever the index is empty.

**What is broken:** If a user clears the sheet (deletes confirmed) and later pastes 20 rows, the commit appends one `SNAPSHOT`, which carries no rows, instead of 20 `ADDED`. A subscriber that only follows `ADDED` and `UPDATED` misses the rows. A subscriber that handles `SNAPSHOT` has to call `ReadStored` again.

**Fix:** Use `snapshot = record.last_seq == 0 && stored.is_empty()`, or store a `snapshotted` flag on the watch record. Add `cycle_refill_emits_added`.

### GT-10. `changed_columns` omits cleared cells

✅ **Status: fixed.** `changed_columns` is the union of the old and new included cells. A name is listed when the value differs or the cell is only on one side. `store_clear_reports_the_column` clears `City` and the `UPDATED` event names it.

**Where:** `store.rs`, `changed_names`.

**Flow:** For `UPDATED`, the function iterates only the new cells and compares each with the stored cell of the same name.

**What is broken:** Empty cells are omitted from the scan. Clearing `City` changes the hash, so `UPDATED` is emitted, but `City` is not in `changed_columns`. The list can even be empty. The design says `UPDATED` lists "the names of the cells that differ".

**Fix:** Take the union of names from the old and new included cells, and report a name when its value differs or it is present on one side only. Add a store test that clears one cell.

### GT-11. `DELETED` rows carry anonymous cells

**Where:** `store.rs`, `row_from_stored`.

**Flow:** A `DELETED` event carries the last stored row. `StoredRow` cells have names. `row_from_stored` turns them into `Cell { excel_id: "" }`.

**What is broken:** A `DELETED` row has values with neither a letter nor a name, so a subscriber cannot tell which value belongs to which column. `ReadStored` fills letters from the watch columns, but the event path does not.

**Fix:** Pass the watch's current columns into `commit`, which already receives `options.columns`, and fill `excel_id` by name, as `read_stored_batches` does. Add the assertion to `store_add_update_delete`.

### GT-12. `MAPPING` is not implemented

**Where:** `watch.rs`, `start_watch` reuses the record for the same spreadsheet and `sheetId`. `store.rs`, `commit`.

**Flow:** `StopWatch`, a new session with a different column set or other exclusions, then `StartWatch`. The `wid` and the LMDB index are reused. The next cycle computes hashes over the new column set.

**What is broken:**

- The design says a mapped-column set that differs from the stored set appends `MAPPING`. Nothing does that.
- Every row's hash differs, so the first cycle appends one `UPDATED` per row. That is a flood, and it is not subject to the mass-delete guard.

**Fix:** In `start_watch`, compare the new columns and exclusions with the record. When they differ, append one `MAPPING` event with the new list and mark the record so the next commit rebases quietly: recompute hashes and store them without row events, or with one `SNAPSHOT`. Add `watch_remap_emits_mapping`.

### GT-13. `StopWatch` does not survive a restart

**Where:** `watch.rs`, `stop_watch` keeps `Inner.stopped` in memory. `resume_spreadsheet` skips only those ids.

**Flow:** `StopWatch` aborts the task. A process restart has an empty `stopped` set, so `insert_book`, or startup in production, resumes every watch record, including stopped ones.

**What is broken:** A watch an operator stopped starts writing UIDs and formulas into the sheet again after a restart.

**Fix:** Persist `status = "stopped"` on the record. Skip that status in `resume_spreadsheet`, and set it back to `running` in `start_watch`. Add `watch_stop_survives_restart`.

### GT-14. Cycle errors are not recorded or backed off

**Where:** `watch.rs`, `watch_loop`, where `run_cycle(...).is_ok_and(...)` ignores the `Err` branch.

**Flow:** Permission denied, a deleted spreadsheet, a deleted sheet tab, or an LMDB error returns `Err`. The loop sets `started = true`, sees `live != baseline`, and runs again 10 seconds later.

**What is broken:** The design's `status` is "running, needs_confirm, column_missing, or the last error". The error is never stored, so `GetWatch` keeps saying `running`. A permanent error retries every 10 seconds forever and spends quota (GT-07).

**Fix:** On `Err`, call `set_status(wid, short reason)` and wait with the backoff exponent before retrying (10 s, 20 s, up to 10 minutes). Reset on a commit. Add `watch_error_sets_status`.

### GT-15. Service-column check is a call on every cycle

**Where:** `cycle.rs`, the two `ensure_service_column` calls before the two-column read.

**Flow:** Every cycle calls `ensure_service_column` for `irbis.uid` and for `irbis.fingerprint`, even when `meta` already lists both.

**What is broken:** In the fake these calls cost nothing. A real implementation has to search metadata or send a `batchUpdate` to honor the trait, which adds two calls to every cycle. The design budget is "one metadata search, one two-column read".

**Fix:** Decide from `meta`, which was just read. Call the append operation only for a key that is absent. Rename the trait method to `append_service_column`, which always appends, so the trait makes no extra call by contract. Count the calls in `cycle_narrow_fetch`.

### GT-16. UID writes are addressed by row number

**Where:** `cycle.rs`: `decide_uids` (writes `(sheet_row, uid_col, uid)`), the `write_raw` call, and the decision fetch inside `decide_uids`.

**Flow:** The cycle reads the columns at version V, checks V, fetches mapped cells for duplicate and restored rows (no version check), then writes UIDs to fixed row numbers. A 429 backoff between those steps can last up to 60 seconds.

**What is broken:** If an editor inserts or deletes a row in that window, the write lands one row off. It can overwrite another row's valid UID. That row's stored UID then looks missing and is emitted as `DELETED`, while the written id is emitted as `ADDED`. The decision fetch can also see cells newer than the snapshot, which leads to a wrong restore or a wrong keeper.

**Fix:** Read `version` again after the decision fetch and immediately before `write_raw`. Drop the cycle without writing if it moved. Never write into a cell that held a valid UID in the snapshot unless that UID is being replaced as a duplicate or an invalid value. Make the fake able to insert a row between calls, and add `cycle_insert_before_write_drops`.

### GT-17. Fingerprint collisions hide real edits

**Where:** `uid.rs`, `fingerprint_formula` (`A&"|"&B`), and `rows_to_fetch`.

**Flow:** A row is fetched only when its fingerprint string changes. The content hash is computed only for fetched rows.

**What is broken:**

- `Name="a|b", City="c"` and `Name="a", City="b|c"` produce the same fingerprint, so that edit is never fetched or reported.
- Sheets concatenation turns the text `"2"` and the number `2` into the same string, so a type change, which does change the hash, is missed.
- A row whose joined text passes the 50,000-character cell limit shows an error value that never changes.

**Fix:** Join with a separator that users do not type, such as `CHAR(31)`, and escape it with `SUBSTITUTE`. Prefix each part with a kind marker: `IF(ISNUMBER(A2:A),"n","t")&A2:A`. Record in design.md that the fingerprint is an approximation, and that `SyncNow` with a full refresh (a new flag) re-hashes every row. Add a `uid_formula_*` test for the separator and the kind marker.

### GT-18. Excluding every mapped column empties the watch

**Where:** `session.rs`, `apply_options` and `SetColumnOptions`. `uid.rs`, `fingerprint_formula` returns `=ARRAYFORMULA("")`. `watch.rs`, `start_watch`.

**Flow:** A caller excludes all mapped columns, then calls `EnsureIdentity` and `StartWatch`.

**What is broken:** No row has a fingerprint, so `read_service_rows` skips every row. The watch sees an empty sheet. A watch that was running holds a 100 percent delete, or deletes everything when fewer than 10 rows are stored. A row whose only values are in excluded columns is treated the same way, as empty.

**Fix:** Reject `SetColumnOptions`, `EnsureIdentity`, and `StartWatch` with `INVALID_ARGUMENT` when no mapped column is included. Separately, decide whether presence should use all mapped columns: `LEN(all)=0` decides empty, and only the joined value skips the excluded ones. Update design.md either way.

### GT-19. Formula comparison is exact text

✅ **Status: fixed.** `irbis.fingerprint` stores a stamp of the formula last written. The cycle rewrites when that stamp differs or the formula cell is empty. `cycle_normalized_formula_commits` rewrites once, then commits, while the fake returns a normalized formula.

**Where:** `cycle.rs`, `if current != expected` before `write_formula`.

**Flow:** The cycle reads the formula in row 1 of the fingerprint column and compares it character by character with `fingerprint_formula(...)`.

**What is broken:** Sheets returns formulas normalized: spacing, case, and sometimes locale separators (`;`) or rewritten references. Any difference makes every cycle rewrite the formula. The write moves `version`, the next cycle rewrites again, and the watch never commits. The fake stores the text as written, so tests cannot show this.

**Fix:** Store the expected formula's hash in the `irbis.fingerprint` metadata value when it is written. Compare that, plus "the cell is not empty", instead of the text. Alternatively, normalize both sides (remove spaces, uppercase outside string literals) before comparing. Add a fake mode that returns a normalized formula, and test that one rewrite is followed by a commit.

### GT-20. Quadratic lookups in the cycle and commit

✅ **Status: fixed.** The cycle builds one UID map and a set of changed rows. A commit carries the previous hash on the upsert, so it does not scan the index again.

**Where:** `cycle.rs`: `rows_to_fetch` (`stored.iter().find` per row), `build_scan` (`changed.contains` per row), `decide_uids` (`missing.iter().position`, `stored.iter().find` per group). `store.rs`, `commit` (`stored.iter().find` per upsert that is not `cells_known`).

**Flow:** Each sheet row looks up its stored entry with a linear scan.

**What is broken:** On a 10,000-row sheet, a first scan or a sort costs about 10⁸ string comparisons per call site, which is seconds of CPU while holding the global cycle mutex (GT-03).

**Fix:** Build `HashMap<&str, &IndexEntry>` once per cycle, and use `HashSet<u32>` for `changed`. In `commit`, use the sorted merge that `plan_merge` already has, and carry the previous hash in the `Upsert`.

### GT-21. Event-log reads scan the whole prefix

✅ **Status: fixed.** `changes_after` reads the oldest key, then seeks to `after + 1`. `trim` deletes from the front and stops at the first event that is inside the retained count and younger than seven days. Each watch has its own `Notify`.

**Where:** `store.rs`, `changes_after` and `trim`. `session.rs`, `Inner.commits: Notify`.

**Flow:**

- `changes_after(wid, after)` iterates every event key for the watch from seq 1 and decodes those above `after`.
- `trim` iterates every key and its `ts` entry after every commit.
- Every commit calls `notify_waiters()` on one `Notify` shared by all watches, so every subscriber of every watch wakes and runs `changes_after`.

**What is broken:** With 100,000 retained events per watch, each wakeup and each commit walks 100,000 keys. With N subscribers across M watches, every commit costs N full scans.

**Fix:** Seek with a range from `event_key(wid, after + 1)` to the end of the prefix, and read the oldest key with `first()` for the `OUT_OF_RANGE` check. In `trim`, delete from the front and stop at the first event that is both inside the count and younger than 7 days. Keep a `Notify` per `wid`.

### GT-22. Subscriber tasks leak after disconnect

**Where:** `watch.rs`, the `subscribe` task loop.

**Flow:** When there are no new events, the task waits on `notified`. It notices a closed client only when `try_send` fails.

**What is broken:** A client that disconnects from a quiet watch leaves its task alive. Because of the shared `Notify` (GT-21), the task wakes on other watches' commits, finds nothing for its watch, and waits again, forever.

**Fix:** Use `tokio::select!` on `notified` and `tx.closed()`, and return when the client closes.

### GT-23. LMDB map size is fixed at 1 GiB

✅ **Status: fixed.** `GOOGLE_TABLES_MAP_SIZE` is the ceiling in bytes. The default is 16 GiB. Startup exits when the file is already larger. At 10 percent free it logs a warning. At 5 percent free, and below that, the warning says the space is critical. `store_map_rejects_a_larger_file` and `store_map_headroom_warns` cover the ceiling and the two bands.

**Where:** `store.rs`, `MAP_SIZE = 1 << 30`.

**Flow:** Every commit writes `idx`, `row`, `evt`, and `ts`. `row` holds full cells, and `evt` holds up to 100,000 events per watch, each with a full row.

**What is broken:** A few dozen busy watches with wide rows reach 1 GiB. LMDB then returns `MDB_MAP_FULL`, and every commit fails. Because of GT-14, that is reported nowhere.

**Fix:** Make the map size configurable (`GOOGLE_TABLES_MAP_SIZE`, default 16 GiB, which is address space, not disk). On `MapFull`, grow with `env.resize()` when no transaction is open and retry once. Surface the error in the watch status.

### GT-24. A watch without a header loses row 1

**Where:** `watch.rs`, `start_watch` (`header_row` is 0 when there is no header). `cycle.rs`, the formula lives in row 1 of the fingerprint column. `google.rs`, `synthesized` skips cells that already hold a value or formula.

**Flow:** Data starts at row 1, but row 1 of the fingerprint column holds the formula.

**What is broken:** In the fake, row 1 never gets a fingerprint, so it is skipped, never gets a UID, and is never watched. In real Sheets, the array formula in row 1 does display row 1's join, but the cycle reads that cell as the formula, so the read depends on the value render option. The behavior is undefined, and no test covers it.

**Fix:** For a sheet without a header, store the formula in a cell outside the data, for example the first row after the last frozen row, or require a header row for a watch. Simplest: reject `StartWatch` without a header, and record that rule in design.md.

## Low

### GT-25. `column_missing` moves the baseline version

**Where:** `store.rs`, `mark_column_missing`.

**What is broken:** The design says the baseline moves "only when that later read has the same version before and after". The pause writes `drive_version` without a commit, so `GetWatch.drive_version` no longer means the last committed version. No data is lost, because the next commit diffs against the index.

**Fix:** Store a separate `paused_version`. Treat `live == paused_version` as caught up in the loop, and clear it on commit.

### GT-26. Five `version` reads per change

**Where:** `watch_loop` reads `live` and `again`. `run_cycle` reads V, `after`, and `after_fetch`.

**What is broken:** A changed file costs five Drive calls, where three would do.

**Fix:** Pass `again` into `run_cycle` as V. Keep the two reads inside the cycle.

### GT-27. A column move refetches every row

**Where:** `uid.rs`, `fingerprint_formula` sorts parts by letter.

**What is broken:** Swapping two mapped columns changes the order of the join, so every fingerprint changes, every row is fetched, and every hash comes out equal with no events. That is a full download for nothing.

**Fix:** Order the joined parts by metadata name. A move still rewrites the formula, because the letters changed, but the values stay the same and nothing is fetched.

### GT-28. New rows are downloaded twice when a UID is missing

**Where:** `decide_uids`, `need_cells` includes every empty-UID row when `missing` is non-empty.

**What is broken:** If any stored row was deleted while new rows were added, the new rows are fetched to look for a restore, get new ids, and are fetched again on the next cycle as `ADDED`.

**Fix:** Fetch only empty-UID rows whose fingerprint equals a missing entry's stored fingerprint. Fall back to fetching all of them only when no fingerprint matches.

### GT-29. UID decisions run on a stale formula

**Where:** `run_cycle` runs `decide_uids` before it checks `current != expected`.

**What is broken:** After a column move, the fingerprints in the read still come from the old formula, so "row has a fingerprint" can be wrong. UIDs may be assigned or skipped based on wrong columns. The next cycle corrects most cases, but duplicate decisions are not undone.

**Fix:** When the formula needs a rewrite, write it and return `Wrote` without deciding UIDs. Decide on the next stable read.

### GT-30. Excluded cells are stored and go stale

**Where:** `store.rs`, `stored_from_scan` keeps excluded cells. `cells_known: false` rows keep the old copy.

**What is broken:** `ReadStored` returns an excluded value, for example `Clock`, from the last time another column changed. A consumer may read it as current.

**Fix:** Drop excluded cells from `row` and from events, or document that they are as of the last update. Dropping them matches "excluded from the hash" more closely.

### GT-31. `SyncReport` hides the outcome

**Where:** `watch.rs`, `report`.

**What is broken:** A `SyncNow` that wrote UIDs, was dropped by a moving version, found `column_missing`, or committed a `SNAPSHOT` reports zeros, the same as "no change". `SyncNow` on a `column_missing` watch returns OK.

**Fix:** Add `string outcome = 6` (`committed`, `wrote`, `dropped`, `held`, `column_missing`) and `bool snapshot = 7` to `SyncReport`. Return `FAILED_PRECONDITION` for `column_missing`.

### GT-32. No jitter, and a restart herd

**Where:** `watch.rs` passes `|| Duration::ZERO` as jitter. `watch_loop` runs a cycle on the first iteration of every watch.

**What is broken:** The design's "plus up to one second of jitter" is absent in production. After a restart, every watch reconciles at the same moment and hits the limiter together.

**Fix:** Use a random jitter of 0–1000 ms in production, and keep zero in tests through the existing injection. Stagger the first reconcile by `wid % 60` seconds, or let the limiter in GT-07 pace it.

### GT-33. `StartWatch` keeps an old status

**Where:** `start_watch` reuses the record and keeps `status` and `pending_deletes` when they are set.

**What is broken:** A watch restarted after `needs_confirm` or `column_missing` reports that status until the next commit, even though the mapping may have changed.

**Fix:** Reset `status = "running"` and `pending_deletes = 0` in `start_watch`. The first cycle sets them again if they still apply.

### GT-34. `FakeBook` differs from Sheets in two places

**Where:** `google.rs`, `synthesized` and `display_at`.

**What is broken:** `synthesized` treats the first cell anywhere on the tab whose formula starts with `=` as the fingerprint formula, so a user `=SUM()` in the fixture would break tests. `display_at` formats a date as `YYYY-MM-DD`, while Sheets concatenation uses the serial number.

**Fix:** Look up the formula in row 1 of the `irbis.fingerprint` column only. Use the serial for dates and the raw number for numbers in `display_at`.

### GT-35. Documentation drift

**Where:** design.md and implementation-plan.md.

**What is broken:**

- The plan's shared rules say "Protobuf conversion happens in `server.py`". The crate is Rust.
- The design layout lists `server.rs` (sessions), `sheets.rs` (the cycle), and `push.rs`. The code has `session.rs`, `cycle.rs`, `google.rs`, `store.rs`, and `watch.rs`, and no `push.rs` yet.
- The design's LMDB table has no `ts` sub-database. The plan's step 5 lists four sub-databases. The code opens five.
- The design does not say that a deleted service column is recreated by the cycle with a `Wrote` stop.

**Fix:** Update those lines when the next fixes land. No behavior change is needed.

## Matches the design

These were rechecked and are correct:

- The two-column read is dropped when `version` moves. The mapped fetch is dropped when `version` moves after it. The baseline is stored only on a commit.
- A UID or formula write returns `Wrote` and commits nothing. The next stable cycle commits.
- The fingerprint chooses the fetch, and the content hash chooses the event. An equal hash on a moved row updates `sheet_row` and emits nothing.
- Excluded columns, from the watch list or `irbis.exclude_hash`, are left out of the formula, the hash, and `changed_columns`.
- An empty UID is restored by hash. An invalid UID is replaced. A duplicate is kept on the earlier matching row. Unmatched copies are deleted under the guard.
- A missing `irbis.column` pauses with `column_missing` and does not bind another letter. A deleted UID or fingerprint column is recreated hidden at the right, with a warning-only range on the UID column and the formula in the fingerprint column.
- Letters on the stored watch follow the current metadata index on every commit.
