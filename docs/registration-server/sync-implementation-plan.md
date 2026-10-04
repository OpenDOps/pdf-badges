# Sync — step 7

Build sequence for [porting.md](porting.md) step 7. The paths and fields are [login-and-token.md](login-and-token.md). The token client and the probe are [token-implementation-plan.md](token-implementation-plan.md). The event file is [db-implementation-plan.md](db-implementation-plan.md). How the sync thread shares the core is [design.md](design.md).

This plan replaces the placeholder `--endpoint` downloads with one queue on the sync thread. The timer probes, then the queue holds the ready downloads and uploads. `RemoteServer` sends them. The queue does not render badges or print. `remote.rs` stays the operator bind.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 7 is done when steps 1 through 12 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Queue](#step-1-queue) | Ready calls wait in one queue. A busy queue is not filled again | done |
| [2. Slots](#step-2-slots) | The queue is `SYNC_IN_FLIGHT` long. Same-path downloads stay in order | done |
| [3. Rank](#step-3-rank) | A waiting download takes a free socket before an upload | done |
| [4. Stop](#step-4-stop) | A dead token or a switched event clears the unsent queue. Sockets use `drop_requests` | done |
| [5. Cursors](#step-5-cursors) | Each event file keeps its own cursors. A failed call does not move them | done |
| [6. Enums](#step-6-enums) | `boxapi/dbenums`, then one `boxapi/dbenum` at a time | done |
| [7. Forms](#step-7-forms) | Config, backgrounds, and ticket barcodes | done |
| [8. Zones and codes](#step-8-zones-and-codes) | Zones, the `info` array, badge art, barcode pools | done |
| [9. Visitors](#step-9-visitors) | `tracked/UniRegUser`, pages of 100, file then memory | done |
| [10. The rest of the downloads](#step-10-the-rest-of-the-downloads) | Managers, scans, and org users, only when that id is set | done |
| [11. Upload registrations](#step-11-upload-registrations) | `boxapi/uploadreg` in batches of 20, then the photos of those ids | done |
| [12. Upload scans and attachments](#step-12-upload-scans-and-attachments) | Scans and lead attachments, except those of a new registration `uploadreg` has not accepted | done |

Shared rules:

- The queue lives in `src/registration_server/sync.rs`. `sync_wake` still pumps, waits on the gate, then probes. An accepted probe lets the wake append ready calls. `POST /api/login` and `POST /api/events/select` do not. The five-second link probe does not.
- The queue holds calls that are ready and not yet on a socket. Its length is the process `sync_in_flight` (the compiled `SYNC_IN_FLIGHT`, default 100, unless `--sync-in-flight` replaced it). A call past that length stays in the planner until a place frees. The planner is the cursors, the form ids, the categories, and the outbox. It is not a second queue.
- That length matches the socket cap. A longer queue cannot send sooner: the extra calls would wait for a socket, and a switch would drop them. A shorter queue leaves a socket idle while another path, or an upload, is already ready. A queued call is the path and its fields. The body is not queued.
- Every send is `RemoteServer`. The wake loads `device_id`, `event_id`, and `project_token`, then releases the credential file before the socket. The timeout is `Link::budget().limit`. Tests pass their own timeout and their own cap. The slot tests use a cap of 2.
- Downloads of the same path are sequential. `boxapi/dbenum`, `boxapi/getconf`, `boxapi/getbg`, `boxapi/ticketbarcodes`, `boxapi/getbadge`, `boxapi/barcodes`, `tracked/UniRegUser`, `tracked/ScanUserRef`, and `tracked/OrgRegUser` each have at most one call in the queue or on a socket. The next call for that path is appended after the previous one commits. Different paths run together and share the sockets.
- A later batch of the same upload path is appended after that batch commits, because the next ids are the ones the server has not accepted yet. `boxapi/uploadphoto` calls for different ids may sit in the queue together. Uploads share the queue with downloads.
- `in_synch` unset means the row still has to go up. It does not say whether the server has seen that person. A row inserted from `tracked/UniRegUser` is already on the server. A row inserted locally as a new registration is not, until its id is in `synch_res`. An edit sets `in_synch` unset and does not change that. `boxapi/uploadscans` and `leadgenapi/upload` leave out a row tied to a new registration that is not yet accepted. Other waiting scans and attachments are sent. A dirty downloaded row does not hold them.
- A free socket takes the oldest download in the queue. An upload is sent when no download is waiting. `tracked/OrgRegUser` is a download, so it is taken before an upload. The [porting table](porting.md#step-7--sync) is the dependency list, not one serial pass.
- The probe's `boxapi/barcodesinfo` is only the auth check. The queue reads `info` later, as its own call, and that call may be on a socket beside other paths.
- At most `sync_in_flight` `RemoteServer` calls are on a socket. The HTTP pool idle cap is that same number. A body is read as a stream. The next parse starts only after the previous parse returns, and only after `Gate::wait_until_idle`.
- Each queued call records the open event id. Before it is sent, the wake reads `CurrentEvent::event_id`. A different id, or an empty holder, discards that unsent call. Sockets are cancelled only by `RemoteServer::drop_requests`. `switch_to` of another id already calls it for the previous id, and then `allow_requests` for the new id. This plan does not add a second way to cancel a socket. Writes go through `CurrentEvent` with the recorded id, so a switch rejects them.
- A tick while this event still has a queued or in-flight call does not append the catalog again and does not probe again. A tick on an empty queue probes, and an accepted probe appends the calls that are ready.
- Cursors are rows in that event's `db.sqlite`, table `sync_cursor` (`name TEXT PRIMARY KEY`, `value TEXT NOT NULL`). A missing name is sent as `0`. The cursor changes only after that call's rows commit. `CurrentEvent::clear` does not delete the file, so the cursors stay. A new file has no cursor rows. Switching does not copy them.
- `TokenRejected` calls `CurrentEvent::clear`, discards the unsent queue, and calls `RemoteServer::drop_requests` for that event id. `clear` is not a switch, so this is the call that cancels those sockets. `NoConnection`, `Status`, `BadResponse`, and any other `error_msg` leave the binding and discard the unsent queue. They do not call `drop_requests` or `clear`. That method also refuses later calls for the id until `allow_requests`, which would block the next tick. Calls already on a socket for a kept token run to their own result.
- `--endpoint` stays on the command line. `sync_once` stays for the skeleton tests. The wake does not call it.
- Tests answer a local stand-in on `127.0.0.1`. They do not call kuprin.su. The project token is not printed. A test that needs an event file uses a temp directory and `CurrentEvent`, and does not write the Scala fixture.
- Step 6 stays green: `cargo test --lib registration_server::remote_server` and `cargo test --lib registration_server::sync::tests::prove`. `cargo test --lib registration_server::current` and `cargo test --test admin_pages select_opens_event` stay green.

## Step 1. Queue

[Back to summary](#summary)

Ready calls wait in one queue. The timer is the loop that already exists. A tick that finds work for the open event does not probe and does not append the catalog again.

### Work

1. After an accepted probe, `sync_wake` owns the queue and does not call `sync_once`.
2. The queue records the open event id. It sends no download and no upload in this step.
3. A second `sync_wake` that starts while the first probe has not returned does not open a socket.
4. `probe_runs_before_the_placeholder_downloads` changes with this step. After the probe, the stand-in sees no `--endpoint` request.

### Test scenarios

| Scenario | Assert |
|---|---|
| `an_accepted_probe_arms_the_queue` | The stand-in sees GET `boxapi/barcodesinfo` and then nothing else. The credential file still has the token. The open event stays. |
| `a_busy_queue_is_not_filled_again` | The first probe is held. A second wake accepts nothing new. After the first response, the stand-in has seen one `boxapi/barcodesinfo`. |
| `a_rejected_probe_leaves_the_queue_empty` | `error_msg` `not_logged_in` still clears the binding, as in the token plan. No later path is requested. |

### Done when

`cargo test --lib registration_server::sync::tests::queue` passes, and `cargo test --lib registration_server::sync::tests::prove` stays green.

## Step 2. Slots

[Back to summary](#summary)

The queue is as long as the socket cap. The tests below use 2. Same-path downloads stay in order. Bodies are parsed one at a time. A local request or a print job holds the next parse.

### Work

1. `RemoteServer` gains `request_bytes`. A JSON body with `error_msg` `not_logged_in` or `no_pattern` is still `TokenRejected`. Any other body, including a body that is not JSON, is the bytes. `request` stays JSON-only.
2. The queue holds at most the passed cap of ready calls. The process cap is `sync_in_flight` (compiled `SYNC_IN_FLIGHT`, default 100; `--sync-in-flight` replaces it). At most that many calls are on a socket. The slot tests pass a cap of 2.
3. Two ready downloads of the same path: the second is not sent until the first response has been parsed. Two ready downloads of different paths may be on a socket together.
4. The bytes of one response are parsed only after `Gate::wait_until_idle`, and only after the previous parse has returned.
5. This step uses two stand-in URLs as stand-ins for later downloads. They are removed when step 6 sends `boxapi/dbenums`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `two_calls_are_in_flight` | Two different paths are ready and the cap is 2. The stand-in accepts the second request before it answers the first. |
| `the_same_endpoint_stays_sequential` | Two downloads of one path are ready and the cap is 2. The stand-in accepts the second only after the first response is parsed. |
| `the_queue_holds_one_cap_of_ready_calls` | Three different paths are ready and the cap is 2. The third request is accepted only after one of the first two completes. |
| `one_body_is_parsed_at_a_time` | Two responses are released together. The second parse starts only after the first parse returns. |
| `a_local_call_holds_the_parse` | The gate is held. Both responses have arrived. For 50 ms neither body is stored. After the guard drops, both are stored, one after the other. |
| `bytes_that_are_not_json_are_returned` | `request_bytes` on HTTP 200 and the body `PK` is `Ok`. The same body through `request` is `BadResponse`. |

### Done when

`cargo test --lib registration_server::sync::tests::slots` passes, and `cargo test --lib registration_server::remote_server::tests::bytes` passes. Step 1 stays green.

## Step 3. Rank

[Back to summary](#summary)

Downloads and uploads share the queue. A free socket goes to a download that is waiting. An upload is sent when no download is waiting.

### Work

1. The wake appends ready downloads and ready uploads into the one queue, then fills sockets from the downloads first.
2. An upload is sent only when one is queued. This step treats that as a test flag, one pretend upload. Later steps append from the file.
3. An empty outbox appends no upload.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_download_takes_the_socket_ahead_of_an_upload` | One download and one upload are queued, and one socket is free. The stand-in receives the download. The upload is received only after that download response is read. |
| `an_empty_outbox_opens_no_socket` | No upload is queued. The stand-in sees the download and does not see an upload path. |
| `uploads_share_the_sockets` | Two sockets are free, no download is queued, and two uploads are queued. Both uploads are on a socket together. |

### Done when

`cargo test --lib registration_server::sync::tests::rank` passes, and steps 1 and 2 stay green.

## Step 4. Stop

[Back to summary](#summary)

A dead token or a switched event discards calls that have not been sent. Calls already on a socket are cancelled by `RemoteServer::drop_requests`, the method the client already has. `switch_to` already calls it.

### Work

1. `TokenRejected` from any call calls `CurrentEvent::clear`, discards the unsent queue, and calls `RemoteServer::drop_requests` for that event id. Those sockets return `Dropped`.
2. `NoConnection`, `Status`, `BadResponse`, and a JSON body whose `error_msg` is anything else leave the binding and discard the unsent queue. They do not call `drop_requests`. Nothing further is appended until the next tick. Sockets already open for that token finish as their own error.
3. On a switch, `switch_to` has already called `drop_requests` for the previous id. This step discards the unsent calls recorded for that id and does not send them. The new event is not filled by the old queue. The next wake probes that event and appends its own calls. `allow_requests` for the new id stays inside `switch_to`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `not_logged_in_clears_the_queue` | The second queued call returns `{"error_msg":"not_logged_in"}`. The holder is empty. The third call is not sent. `device_id` and `base_url` stay. `db.sqlite` stays. |
| `another_error_clears_the_queue_and_keeps_the_token` | The second call returns `{"error_msg":"no_barcodes"}`. The token and the open event stay. The third call is not sent. |
| `a_switched_event_clears_the_previous_queue` | The queue records `AAA` and one `AAA` call is on a socket. After `switch_to("BBB")` that call returns `Dropped`. The unsent `AAA` call is not sent. `BBB` has no new request from this queue. |
| `the_next_wake_runs_the_new_event` | After that switch, the next wake sends `boxapi/barcodesinfo` with `eid` `BBB`. |

### Done when

`cargo test --lib registration_server::sync::tests::stop` passes, and steps 1–3 stay green.

## Step 5. Cursors

[Back to summary](#summary)

A call sends the cursor stored for its name. The stored value moves only after the rows from that response commit.

### Work

1. `sync_cursor` is created with the event file. `CurrentEvent::open` on a file that lacks the table adds it. Existing visitor rows stay.
2. A missing name is sent as `0`.
3. After a successful commit the row is the cursor value from that response. A `TokenRejected`, `NoConnection`, `Status`, or `BadResponse` leaves the row unchanged.
4. `switch_to` of another id does not copy `sync_cursor`. A new file has no rows.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_missing_cursor_is_sent_as_zero` | The stand-in sees `last_synch=0` on the call under test. |
| `a_commit_stores_the_cursor` | The response carries `last_synch` `15`. After the commit, `sync_cursor` for that name is `15`. The next call sends `15`. |
| `a_failed_call_keeps_the_cursor` | The stored value is `15`. The call is HTTP 500. The row is still `15`. |
| `a_new_event_has_no_cursors` | `AAA` has cursor `15`. After `switch_to("BBB")` on a missing file, `BBB` has no `sync_cursor` rows. `AAA`'s row is still `15`. |

### Done when

`cargo test --lib registration_server::sync::tests::cursors` passes, and `cargo test --lib registration_server::event_db::tests::open` stays green. Steps 1–4 stay green.

## Step 6. Enums

[Back to summary](#summary)

`boxapi/dbenums` is queued with the other independent downloads. `boxapi/dbenum` is the same path, so those calls go one at a time after `dbenums` commits. The pretend calls from steps 2 and 3 are gone.

### Work

1. The wake queues POST `boxapi/dbenums` with `enums`, `version`, and `langs`. `version` is the `dbenums` cursor. Other ready paths may be on a socket at the same time.
2. After that body commits, the wake queues one `boxapi/dbenum` for the first dependency, with `enum`, `langs`, `gzipped`, and that dependency's ids. The next dependency is queued after that call commits.
3. The `dbenums` cursor moves to the response `version` when that body commits. Other paths are not held for the `dbenum` calls.

### Test scenarios

| Scenario | Assert |
|---|---|
| `dbenums_is_queued_with_the_other_downloads` | `boxapi/dbenums` is on a socket while another ready download is also on a socket. The form has `enums`, `version`, `langs`, `dev_id`, `token`, and `eid`. |
| `dbenum_follows_each_dependency` | The `dbenums` body names two dependencies. The stand-in then sees two `boxapi/dbenum` calls, in that order, each with `enum` and `gzipped`. The second is accepted only after the first response is parsed. |
| `the_version_moves_when_dbenums_commits` | `version` in the response is `3`. `sync_cursor` name `dbenums` is `3` when that body has been stored, before the second `dbenum` returns. |

### Done when

`cargo test --lib registration_server::sync::tests::enums` passes, and steps 1–5 stay green.

## Step 7. Forms

[Back to summary](#summary)

Config, backgrounds, and ticket barcodes. The first `getconf` uses an empty form id. Each form id from that config is then fetched on its own path, and calls of the same path stay in order.

### Work

1. Queue POST `boxapi/getconf` with `last_synch`, `last_cnfids_synch`, and `conf_id` empty.
2. After that body commits, for each form id: queue `boxapi/getconf` with that `conf_id`, `boxapi/getbg` with `resolution` `pad` and that `conf_id`, and `boxapi/ticketbarcodes` with `conf_id` and `clean_id`. The next `getconf` waits until the previous `getconf` commits. The next `getbg` (`phone`, then the next form) waits until the previous `getbg` commits. The next `ticketbarcodes` waits the same way. The three paths run together.
3. `getbg` uses `request_bytes` when the body is not JSON. Ticket barcode rows go into `ticketbarcodes`. The cursors move after each commit.

### Test scenarios

| Scenario | Assert |
|---|---|
| `getconf_starts_with_an_empty_form` | The first `boxapi/getconf` has `conf_id` empty and `last_synch` from `sync_cursor`. |
| `each_form_gets_backgrounds_and_tickets` | The config names form `9`. The stand-in then sees `getconf` with `conf_id=9`, `getbg` `pad`, then `getbg` `phone`, and `ticketbarcodes` with `conf_id=9`. `getbg` `phone` is accepted only after `pad` is parsed. |
| `ticket_barcodes_land_in_the_file` | One barcode in the body is one `ticketbarcodes` row for that form. A later tick does not insert it again. |

### Done when

`cargo test --lib registration_server::sync::tests::forms` passes, and steps 1–6 stay green.

## Step 8. Zones and codes

[Back to summary](#summary)

Zones, the barcode catalog, badge art, and barcode pools. This is where the queue reads `info`. The probe still does not.

### Work

1. Queue POST `boxapi/control_zones` with `last_synch`. It may be on a socket beside other paths.
2. Queue GET `boxapi/barcodesinfo` with no caller fields. Read `info`. This call is not the probe. A `TokenRejected` here still clears the queue.
3. After the categories are known, queue POST `boxapi/getbadge` with `last_synch`, `cat_id`, and `is_cert`, one category at a time, then once for certificates. The body is bytes.
4. After `info` is read, queue POST `boxapi/barcodes` with `cat_id`, `take`, `bc_type`, and either `cert_id` or `multi_day=true`, one pool at a time. Rows go into `barcodes`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `info_is_not_the_probe` | The probe and this GET are two requests. The second one is the first time `info` is read. `control_zones` may be on a socket beside that GET. |
| `badges_follow_the_categories` | Two categories are known. The stand-in sees two `getbadge` calls, in that order. The second is accepted only after the first response is parsed. `barcodes` is not sent before `info` has been read. |
| `barcode_rows_land_in_the_file` | One pool entry is one `barcodes` row with that `cat_id` and `clean_id`. The cursor moves after the insert. |

### Done when

`cargo test --lib registration_server::sync::tests::codes` passes, and steps 1–7 stay green.

## Step 9. Visitors

[Back to summary](#summary)

Visitor pages. The file commits first. Memory is updated from that commit, through `CurrentEvent::write` with the recorded event id. Pages of this path stay in order and may be on a socket beside other paths.

### Work

1. Queue POST `tracked/UniRegUser` with `from_time`, `json` `{event_id}_reg_track_request`, and `limit` `100`.
2. Each visitor is `CurrentEvent::write` for the recorded event id. A page shorter than 100 ends this path. A full page appends the next page with the new `from_time` after this page commits.
3. A switch that lands before a write rejects that write. The cursor does not move past a page that did not commit.

### Test scenarios

| Scenario | Assert |
|---|---|
| `visitors_are_pages_of_100` | The first call sends `limit=100` and `from_time=0`. The body has 100 visitors. The next call sends the `from_time` from that body and is accepted only after the first response is parsed. A body of 40 visitors is the last call. |
| `the_file_is_written_before_memory` | After the page commits, `mem_u` has the uid and `search` for that surname returns it. `loaded_count` includes it. |
| `a_switched_event_does_not_take_the_page` | The holder becomes `BBB` before the write. The write is `NotOpen`. `AAA`'s `from_time` cursor stays. `BBB` has no such visitor. |

### Done when

`cargo test --lib registration_server::sync::tests::visitors` passes, and steps 1–8 stay green.

## Step 10. The rest of the downloads

[Back to summary](#summary)

Managers, scan refs, and org users. Each one is queued only when its id is set. Org users are a download, so a waiting upload does not take the socket ahead of them.

### Work

1. Queue POST `boxapi/getmanagers` with `last_synch` and `orgid` only when an org id is stored for this event. Otherwise the stand-in does not see that path.
2. Queue POST `tracked/ScanUserRef` with `from_time`, `json` `{event_id}_regscan_track_request_short`, `extraId` `zone_{zoneId}`, and `limit` `100`, only when a zone id is stored. Pages follow the visitor rule. Rows go into `scans`.
3. Queue POST `tracked/OrgRegUser` with `from_time`, `json`, `limit` `100`, and `extraId` the org id, only when an org id is stored. While this call is queued, `boxapi/uploadreg` waits.

### Test scenarios

| Scenario | Assert |
|---|---|
| `managers_are_skipped_without_an_org` | No org id is stored. The stand-in does not see `boxapi/getmanagers` or `tracked/OrgRegUser`. |
| `scans_follow_the_zone` | Zone `4` is stored. The stand-in sees `tracked/ScanUserRef` with `extraId=zone_4` and `limit=100`. One row is in `scans`. |
| `org_users_are_taken_before_the_upload` | An org id is stored and one registration is waiting. Both are queued. The stand-in sees `tracked/OrgRegUser` before `boxapi/uploadreg`. |

### Done when

`cargo test --lib registration_server::sync::tests::rest` passes, and steps 1–9 stay green.

## Step 11. Upload registrations

[Back to summary](#summary)

Local registrations that are not yet synced. Twenty at a time. Photos of the ids the server just accepted are appended next, and those photo calls may wait in the queue together.

### Work

1. The next `boxapi/uploadreg` is the `mem_u` rows whose `in_synch` is not set, up to 20. No such row means that path is not queued. The following batch is appended after `synch_res` commits.
2. POST `boxapi/uploadreg` with `vals`, the JSON text of those rows. On `synch_res`, those ids are marked synced and `in_synch` is set. A new local registration is then on the server, so its scans and attachments may go up. Ids absent from `synch_res` stay unsent and stay off the server.
3. For each accepted id that has `{event}/photos/uids/{id}.png`, queue multipart `boxapi/uploadphoto` with that file and field `f_{id}`. Several of those calls may be in the queue together. A missing file is not a part and does not fail the batch.

### Test scenarios

| Scenario | Assert |
|---|---|
| `no_waiting_row_skips_uploadreg` | Every `mem_u` row has `in_synch` set. The stand-in does not see `boxapi/uploadreg`. |
| `twenty_waiting_rows_are_one_batch` | 21 rows are waiting. The first `vals` array has 20. Those 20 have `in_synch` set after `synch_res`. The 21st is the next batch, accepted only after the first response is parsed. |
| `photos_follow_the_accepted_ids` | Two ids are accepted and both have a png. The stand-in sees two multipart `boxapi/uploadphoto` calls, and with two free sockets both are in flight together. An id without a file is not a part. |

### Done when

`cargo test --lib registration_server::sync::tests::upload_reg` passes, and steps 1–10 stay green.

## Step 12. Upload scans and attachments

[Back to summary](#summary)

Scans and lead attachments. A row is sent when it is waiting and its registration is already on the server. A new local registration that `uploadreg` has not accepted holds only its own scans and attachments. A downloaded registration with `in_synch` unset does not. Both paths are skipped when nothing remains to send. The next batch of a path is appended after that batch commits.

### Work

1. Queue POST `boxapi/uploadscans` with `vals` for up to 100 `scans` rows whose `synched` is not set, leaving out a scan whose barcode belongs to a new registration that is not yet accepted. Accepted ids get `synched` set. An empty list does not queue the path. The next batch waits for this commit. A scan held back is eligible after `synch_res` accepts that registration.
2. Queue POST `leadgenapi/upload` with `vals`, `no_need_data=true`, and `expoId` the recorded event id, for up to 20 attachments waiting, leaving out an attachment whose `man_to_user.uid` is a new registration that is not yet accepted. Accepted ids are marked sent. This path may be on a socket beside `uploadscans` when no download is waiting.
3. A `TokenRejected` on either call still clears the queue and the binding. The ids not yet accepted stay unsent.

### Test scenarios

| Scenario | Assert |
|---|---|
| `scans_go_up_in_batches_of_100` | 101 unsent scans, none tied to a new registration. The first `vals` has 100. Those 100 have `synched` set. The 101st is the next batch, accepted only after the first response is parsed. |
| `no_scans_skips_the_call` | Every scan is `synched`. The stand-in does not see `boxapi/uploadscans`. |
| `a_new_registration_holds_only_its_own_rows` | One registration was inserted locally and is not in `synch_res`. One scan and one attachment belong to it, and one scan belongs to a downloaded registration whose `in_synch` is unset. The stand-in sees the downloaded registration's scan. It does not see the new registration's scan or attachment. After `synch_res` accepts that id, the next batch includes them. |
| `attachments_send_the_event_id` | One attachment is waiting. The stand-in sees POST `leadgenapi/upload` with `no_need_data=true` and `expoId` the open event id. |
| `a_rejected_upload_leaves_the_rest_unsent` | The scans call returns `not_logged_in`. The binding is cleared. `leadgenapi/upload` is not called. The scans stay unsent. |

### Done when

`cargo test --lib registration_server::sync::tests::upload_rest` passes. `cargo test --lib registration_server::sync`, `cargo test --lib registration_server::remote_server`, `cargo test --lib registration_server::current`, and `cargo test --test admin_pages select_opens_event` stay green.
