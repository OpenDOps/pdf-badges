# Switch, load, or init — step 5

Build sequence for [porting.md](porting.md) step 5. The file and the memory copy are [db-implementation-plan.md](db-implementation-plan.md). Choosing an event is [admin-implementation-plan.md](admin-implementation-plan.md) step 11.

This plan opens the event named in the credential file, at startup and after a successful select. A missing file is created. An existing file is copied into memory in the background. It does not send the project token, download visitors, or edit operator keys.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 5 is done when steps 1 through 6 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Bound open](#step-1-bound-open) | Startup opens only when the token and the id are both stored | done |
| [2. Init](#step-2-init) | A missing file gets the step 4 schema and is `ready` | done |
| [3. Background copy](#step-3-background-copy) | One batch per sync wake, first 100 in `last_add` order | done |
| [4. Switch](#step-4-switch) | The next file opens before the previous pair closes | done |
| [5. Select](#step-5-select) | `POST /api/events/select` opens after the token is stored | done |
| [6. Clear](#step-6-clear) | Clearing the binding closes the file and the memory database | done |

Shared rules:

- The holder is `CurrentEvent` in `src/registration_server/current.rs`. It owns at most one `EventDb`. The server thread and the sync thread share it behind a mutex. A batch holds that mutex, then releases it, so a search waits out one batch and not the whole copy.
- The event directory is the directory of the credential file. `base/credentials.yml` opens `base/{event_id}/db.sqlite`. Tests pass a temp directory and put the credential file in it.
- `open` still does not copy rows. `CurrentEvent::pump` calls `load_batch` once. The sync loop calls `pump` on each wake, before the endpoint downloads. This plan does not change those downloads.
- Search, the list page, a write, and `full_row` use the open `EventDb`. Search reads committed memory rows. The list reads the file until `index_state` is `ready`, then reads memory. Those functions are already tested on `EventDb`. The tests here prove the holder and the select route call them.
- A select test uses the local stand-in from `tests/admin_pages.rs`. It does not call kuprin.su. A file test copies the Scala event and does not write the original:

```text
/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite
```

- Step 4 stays green: `cargo test --lib registration_server::event_db`.

## Step 1. Bound open

[Back to summary](#summary)

Startup opens the event only when the credential file is already logged in.

### Work

1. `CurrentEvent::open_bound` reads the credential file. `event_id` and `project_token` both set calls `EventDb::open`. Otherwise the holder stays empty.
2. It does not call `auth/login`, `boxapi/expos`, or `profile/synchdev`.
3. `run` calls `open_bound` after the credential file loads, before the sync thread starts.

### Test scenarios

| Scenario | Assert |
|---|---|
| `startup_opens_a_bound_file` | A credential file with `event_id` and `project_token`, and no `db.sqlite`, leaves that id open. `index_state` is `ready`. No HTTP client was built for the test. |
| `startup_without_a_token_opens_nothing` | `event_id` without `project_token` is refused by the credential file. A file with neither field leaves the holder empty and creates no `db.sqlite`. |

### Done when

`cargo test --lib registration_server::current::tests::open_bound` passes.

## Step 2. Init

[Back to summary](#summary)

A bound id with no database file gets an empty event.

### Work

1. `open_bound` on a missing `db.sqlite` uses the step 4 create path. The schema, the two `keys` rows, and `index_state` `ready` are that path.
2. `loaded_count` is `0`. `pump` returns false.

### Test scenarios

| Scenario | Assert |
|---|---|
| `missing_file_is_ready` | After `open_bound`, `db.sqlite` exists, `keys` has one admin and one operator, `loaded_count` is `0`, and `index_state` is `ready`. |
| `pump_on_an_empty_file_is_idle` | `pump` returns false and `loaded_count` stays `0`. |

### Done when

`cargo test --lib registration_server::current::tests::init` passes, and step 1 stays green.

## Step 3. Background copy

[Back to summary](#summary)

An existing file stays `loading` until `pump` has copied it. The first wake copies the first five list pages.

### Work

1. `open_bound` on a file that already has visitors leaves `index_state` `loading` and `loaded_count` `0`. It does not call `load_batch`.
2. `pump` calls `load_batch` once and returns what that call returns. The first call inserts 100 rows in the table `last_add` order, or fewer when the file is smaller.
3. The sync loop calls `pump` once per wake when an event is open. It does not hold the mutex across the endpoint downloads.
4. `pump` on a `ready` event returns false and does not insert.

### Test scenarios

| Scenario | Assert |
|---|---|
| `open_does_not_copy` | After `open_bound` on the copied fixture, `loaded_count` is `0` and `index_state` is `loading`. |
| `first_pump_is_five_pages` | One `pump` leaves 100 rows. The order matches `table` `last_add` for `from` 0 and `limit` 100: uid 733, then 3534 at row 20, 3535 at row 21, 795 last. uid 796 is absent. |
| `pump_stops_when_ready` | After the copy finishes, `index_state` is `ready`, `loaded_count` is 5176, and another `pump` returns false. |
| `search_sees_the_first_pages` | After the first `pump`, search for `агранович` returns uid 733. Search for `утимото` returns nothing until a later `pump` has copied uid 796. |

### Done when

`cargo test --lib registration_server::current::tests::pump` passes, and steps 1 and 2 stay green.

## Step 4. Switch

[Back to summary](#summary)

Choosing another event opens that file before the previous pair is dropped.

### Work

1. `switch_to` opens the next id. On success the holder replaces the current `EventDb`. The previous memory database closes with that drop.
2. Rows are not copied from the previous file into the next one.
3. A failed open leaves the current `EventDb` in place. An id that is not one path component fails before a file is created.
4. `switch_to` the id that is already open is a no-op. The memory rows stay.

### Test scenarios

| Scenario | Assert |
|---|---|
| `switch_opens_the_next_before_dropping` | Event `AAA` has one visitor. After `switch_to("BBB")` on a missing file, `BBB` is `ready` and its list is empty. `AAA/db.sqlite` still has that visitor. |
| `switch_does_not_copy_rows` | A visitor written on `AAA` is absent from `BBB` memory and from `BBB/db.sqlite`. |
| `failed_switch_keeps_the_current_file` | `switch_to("../x")` returns `BadExpoId`. The holder is still `AAA` and still has its visitor. No `../x` directory is created. |
| `switch_to_the_same_id_keeps_memory` | A row in memory is still there after `switch_to` of the current id. |

### Done when

`cargo test --lib registration_server::current::tests::switch` passes, and steps 1–3 stay green.

## Step 5. Select

[Back to summary](#summary)

The route step 3 already serves writes the token, then switches. The route stays `POST /api/events/select`.

### Work

1. An id that is empty, `.`, `..`, or contains `/`, `\`, or a null byte is `400` `bad_request`. The stand-in is not called. The credential file and the open event stay as they were.
2. After `OperatorSession::bind` succeeds, `switch_to` runs for that id. The JSON body stays `{ "id", "name", "token_stored": true, "timeout_secs" }`. The token string is not in the JSON.
3. A bind failure does not open a file and does not replace the current event.
4. If the token is stored and `switch_to` then fails, the response is `500` `unknown_error`. The previous event stays open.

### Test scenarios

| Scenario | Assert |
|---|---|
| `select_opens_the_file` | After a successful select, `base/{id}/db.sqlite` exists, the holder’s id is that id, and `index_state` is `ready`. `GET /api/binding` still reports `token_stored` true. |
| `select_replaces_the_previous_file` | A visitor on the first id is still in that file after a second select. The holder’s id is the second id, and its list is empty. |
| `select_bind_failure_does_not_open` | Stand-in synchdev returns HTTP 500. No `db.sqlite` is created for that id. An event that was already open is still open. |
| `select_rejects_a_bad_id` | Body `id` `../x` is `400` `bad_request`. The stand-in saw no `profile/synchdev`. The credential file is unchanged. |

### Done when

`cargo test --test admin_pages select_opens_event` passes, and steps 1–4 stay green. `cargo test --test admin_pages select_event` stays green.

## Step 6. Clear

[Back to summary](#summary)

Clearing the binding closes the open pair. The file stays on disk.

### Work

1. `clear` calls `Credentials::clear_binding`, stores the credential file, and drops the `EventDb`.
2. The `db.sqlite` file is not deleted. A later `open_bound` with a new token opens it again, including its visitors.
3. `pump` after `clear` returns false and does not insert.
4. There is no new HTTP route in this step. Step 6 of [porting.md](porting.md) is what calls `clear` when the server rejects the token.
5. SIGINT and SIGTERM stop the server thread and the sync thread, then close the open database. The credential file stays as it is, so the next start opens the same event.

### Test scenarios

| Scenario | Assert |
|---|---|
| `clear_closes_the_pair` | After `clear`, the holder is empty. `pump` returns false. `db.sqlite` is still on disk and still has its visitor. |
| `clear_keeps_device_and_url` | The credential file still has `device_id` and `base_url`. `event_id`, `event_name`, and `project_token` are absent. |
| `open_again_loads_the_same_file` | Bind the same id again and `open_bound`. The visitor is still in the file. `index_state` is `loading` until `pump` copies it. |

### Done when

`cargo test --lib registration_server::current` passes, and `cargo test --test admin_pages select_opens_event` stays green.
