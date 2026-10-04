# Working with tokens — step 6

Build sequence for [porting.md](porting.md) step 6. The wire fields are [login-and-token.md](login-and-token.md). Opening and closing the event file is [switch-implementation-plan.md](switch-implementation-plan.md). The timeout comes from [network-quality.md](network-quality.md).

This plan adds the remote-server client: the module that calls kuprin with a stored project token. It sends `dev_id`, `token`, and `eid`, probes that token with `GET boxapi/barcodesinfo`, and the sync wake clears the binding when the client reports the bind is dead. It does not parse `info`, download visitors, or upload rows. A zip or octet-stream body is step 7, on this same client.

`remote.rs` stays the operator bind. Login, the event list, and `profile/synchdev` keep `JSESSIONID` and the single-flight flag. This client does not use that session.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 6 is done when steps 1 through 6 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Form](#step-1-form) | `RemoteServer` POST form carries the caller fields plus `dev_id`, `token`, and `eid` | done |
| [2. GET](#step-2-get) | The same fields go in the query | done |
| [3. Multipart](#step-3-multipart) | Files plus the same fields | done |
| [4. Outcome](#step-4-outcome) | `not_logged_in` and `no_pattern` are a rejected token. Any other body keeps it | done |
| [5. Probe](#step-5-probe) | The sync wake asks the client for `boxapi/barcodesinfo` before the placeholder downloads | done |
| [6. Drop](#step-6-drop) | A rejected token clears the binding, closes the event, and skips the rest of the wake | done |

Shared rules:

- The module is `registration_server::remote_server` in `src/registration_server/remote_server.rs`. The type is `RemoteServer`. `new` takes the remote base, the same string as `Remote::new`. It owns its `reqwest::Client`. One client is built at startup from `Config.remote_base` and shared by the sync thread and the open event.
- `request(method, path, fields, auth, timeout)` returns the JSON body. `auth` is `device_id`, `event_id`, and `project_token`. The client does not read the credential file and it does not close the event. Each call is registered under that `event_id` before the socket. `drop_requests(event_id)` cancels the calls still running for that id. A later call for it is `Dropped` and opens no socket, until a switch back to that id. `Dropped` does not clear the binding. `switch_to` of a different id calls `drop_requests` for the previous id after the next file is open. The same id does not. A failed switch does not.
- A missing `device_id`, `event_id`, or `project_token`, including a whitespace-only value, is `RemoteServerError::InvalidCred`. The stand-in sees no connection. The credential file is not written.
- The three fields are written after the caller fields, so a caller field named `dev_id`, `token`, or `eid` is replaced by the stored value. The client does not send `JSESSIONID`. It does not take the operator single-flight flag, so a probe can be in flight while login, the event list, or bind is in flight.
- `RemoteServerError::TokenRejected` is `error_msg` `not_logged_in` or `no_pattern`. `RemoteError::SessionExpired` stays on the operator client, for the cookie failure `You are not logged in!`. This client does not return `SessionExpired`, and `remote.rs` does not gain `TokenRejected`.
- The sync wake in `src/registration_server/sync.rs` calls `RemoteServer::request` after `pump` and before `sync_once`. `POST /api/login` and `POST /api/events/select` do not call it. The five-second link probe does not call it.
- The wake loads the credential file, then releases it before the socket. It sends only when `is_logged_in` is true and the open event id is that same id. The timeout is `Link::budget().limit`. Tests pass their own timeout.
- A rejected token calls `CurrentEvent::clear` from the switch plan. That drops the open pair and removes `event_id`, `event_name`, and `project_token`. The `db.sqlite` file stays on disk. `device_id` and `base_url` stay. This plan does not add a route that clears the binding.
- The placeholder `--endpoint` downloads stay on the sync client they already use. `sync_once` does not attach a token. An accepted probe is what lets the wake call `sync_once`. Step 7 replaces those downloads with `RemoteServer` calls.
- Tests answer a local stand-in on `127.0.0.1`. They do not call kuprin.su. The project token is not printed. A test that needs an event file uses a temp directory and `CurrentEvent`, and does not write the Scala fixture.
- Step 5 stays green: `cargo test --lib registration_server::current` and `cargo test --test admin_pages select_opens_event`. The operator client stays green: `cargo test --lib registration_server::remote`.

## Step 1. Form

[Back to summary](#summary)

POST is the default. The body is `application/x-www-form-urlencoded`.

### Work

1. `RemoteServer::request` joins the path to the base. `boxapi/getconf` on `http://127.0.0.1:port/` is that path under the base.
2. The body contains the caller fields and `dev_id`, `token`, and `eid`.
3. An empty credential returns `InvalidCred` before `reqwest` is asked to send.

### Test scenarios

| Scenario | Assert |
|---|---|
| `post_sends_the_three_fields` | Caller fields are `last_synch=0`. The stand-in sees POST `boxapi/getconf`, content type `application/x-www-form-urlencoded`, and form fields `last_synch`, `dev_id`, `token`, and `eid` with the stored values. The request has no `Cookie` header. The JSON body is returned. |
| `stored_token_replaces_a_caller_token` | A caller field `token=other` arrives as the stored token. |
| `missing_credential_opens_no_socket` | An empty `device_id`, an empty `event_id`, and an empty `project_token` are each `invalid_cred`. The stand-in accepts nothing. |

### Done when

`cargo test --lib registration_server::remote_server::tests::form` passes.

## Step 2. GET

[Back to summary](#summary)

GET puts the same fields in the query string. `boxapi/barcodesinfo` is the call that uses it.

### Work

1. The method is GET. There is no form body.
2. The query carries the caller fields and `dev_id`, `token`, and `eid`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `get_puts_the_fields_in_the_query` | Caller field `last_synch=1` is on the query with `dev_id`, `token`, and `eid`. The method is GET. The body is empty. |
| `get_missing_credential_opens_no_socket` | An empty token is `invalid_cred`. The stand-in accepts nothing. |

### Done when

`cargo test --lib registration_server::remote_server::tests::get` passes, and step 1 stays green.

## Step 3. Multipart

[Back to summary](#summary)

Multipart sends files and keeps the form fields. `boxapi/uploadphoto` is the call that uses it. This step does not decide which photos exist.

### Work

1. The method takes file paths plus the caller fields. Each file is a part. The fields, including `dev_id`, `token`, and `eid`, are parts too.
2. A path that is not a file returns an error and does not open a socket.

### Test scenarios

| Scenario | Assert |
|---|---|
| `multipart_sends_the_file_and_the_fields` | One file and caller field `f_1=uid`. The stand-in sees `multipart/form-data`, the file bytes, and parts `f_1`, `dev_id`, `token`, and `eid`. |
| `missing_file_opens_no_socket` | A path that is not a file does not reach the stand-in. |

### Done when

`cargo test --lib registration_server::remote_server::tests::multipart` passes, and steps 1 and 2 stay green.

## Step 4. Outcome

[Back to summary](#summary)

The client reads the auth outcome and nothing else about the path. The check runs on the response of every method. These tests use POST.

### Work

1. HTTP 200 and a JSON object is success, including a body that has some other `error_msg`. The value is returned to the caller.
2. HTTP 200 and `error_msg` `not_logged_in` or `no_pattern` is `RemoteServerError::TokenRejected`. The credential file is not changed in this step.
3. Any other HTTP status is `Status`. A connect failure or a timeout is `NoConnection`. A 200 body that is not JSON is `BadResponse`. Each of those keeps the token for a later step to observe: this step only returns the error.
4. The client does not look at `info`, `status`, or `arr`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `other_error_msg_returns_the_body` | `{"error_msg":"no_barcodes","info":[1]}` is `Ok`. The `info` value is still in the JSON. |
| `not_logged_in_is_rejected` | `error_msg` `not_logged_in` is `TokenRejected`. |
| `no_pattern_is_rejected` | `error_msg` `no_pattern` is `TokenRejected`. |
| `http_error_is_status` | HTTP 500 is `Status(500)`. |
| `timeout_is_no_connection` | A stand-in that accepts and does not answer, with a short timeout, is `NoConnection`. |
| `non_json_is_a_bad_response` | HTTP 200 and the body `ok` is `BadResponse`. |

### Done when

`cargo test --lib registration_server::remote_server::tests::outcome` passes, and steps 1–3 stay green.

## Step 5. Probe

[Back to summary](#summary)

The sync thread is the caller. It probes after the event file is open, and before the placeholder downloads. The client stays a request: the wake decides when to send `boxapi/barcodesinfo`.

### Work

1. The wake loads the credential file. Logged out, or an open event whose id is not the stored id, returns before a socket. The open event stays as it was.
2. Otherwise it calls `RemoteServer::request` with GET `boxapi/barcodesinfo`, no caller fields, and the given timeout. HTTP 200 JSON without `not_logged_in` or `no_pattern` is accepted. `info` is not read.
3. The sync wake waits on the gate, then sends that probe, then calls `sync_once` only when the probe is accepted. `sync_loop` passes `Link::budget().limit`.
4. `NoConnection`, `Status`, and `BadResponse` leave the binding and the open event in place, and the wake does not call `sync_once`.
5. Select stores the token and switches. It does not request `boxapi/barcodesinfo`. The next sync wake does.

### Test scenarios

| Scenario | Assert |
|---|---|
| `probe_accepts_barcodesinfo` | The stand-in returns `{"info":[{"cat_id":1}]}`. The wake accepts it. The request is GET `boxapi/barcodesinfo` with `dev_id`, `token`, and `eid`, and no other query field. The credential file still has the token. |
| `unbound_probe_opens_no_socket` | A credential file with `device_id` and `base_url` only, and no open event, does not reach the stand-in. |
| `probe_waits_for_a_different_open_event` | The file names event `BBB` while `AAA` is open. The stand-in accepts nothing. `AAA` stays open and the token stays in the file. |
| `probe_runs_before_the_placeholder_downloads` | One wake against a stand-in that accepts the probe and serves one `--endpoint` URL. The first request is `boxapi/barcodesinfo`. The endpoint request is second. The endpoint request has no `dev_id`. |
| `a_local_call_holds_the_probe` | The gate is held. For 50 ms the stand-in has accepted nothing. After the guard drops, the probe completes. |
| `select_does_not_probe` | After `POST /api/events/select` the stand-in has seen `profile/synchdev` and has not seen `boxapi/barcodesinfo`. |

### Done when

`cargo test --lib registration_server::sync::tests::prove` passes, and `cargo test --test admin_pages select_does_not_probe` passes. Steps 1–4 stay green.

## Step 6. Drop

[Back to summary](#summary)

A rejected probe deletes the binding and closes the event. Later wakes do not call the remote server until the operator binds again. `RemoteServer` only reports `TokenRejected`. The wake is what clears.

### Work

1. `TokenRejected` from the probe calls `CurrentEvent::clear`, then the wake returns. `sync_once` is not called.
2. `clear` removes `event_id`, `event_name`, and `project_token` and drops the open pair. `device_id`, `base_url`, and `db.sqlite` stay.
3. The next wake sees a logged-out file and does not open a socket.
4. `no_barcodes`, HTTP 500, and `NoConnection` do not call `clear`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `not_logged_in_clears_the_binding` | The probe body is `{"error_msg":"not_logged_in"}`. The holder is empty. The file has `device_id` and `base_url` and does not have `event_id`, `event_name`, or `project_token`. `db.sqlite` is still on disk and still has its visitor. The placeholder endpoint was not requested. |
| `no_pattern_clears_the_binding` | `error_msg` `no_pattern` does the same clear. |
| `the_next_wake_opens_no_socket` | After that clear, another wake does not reach the stand-in. |
| `other_error_keeps_the_token` | `{"error_msg":"no_barcodes"}` leaves the token and the open event. The placeholder endpoint is still requested, because the probe was accepted. |
| `a_dead_socket_keeps_the_token` | Nothing is listening. The probe is `NoConnection`. The token and the open event stay. The placeholder endpoint is not requested. |

### Done when

`cargo test --lib registration_server::sync::tests::prove` passes, and `cargo test --lib registration_server::remote_server` passes. `cargo test --lib registration_server::remote`, `cargo test --lib registration_server::current`, and `cargo test --test admin_pages select_opens_event` stay green.
