# event-sync

One library keeps an event file in step with the remote server. A host binds a store, a transport, and a catalog. The library runs the queue. The registration desk, an access-control station, a phone that scans a hall, and a door reader are hosts of that same library. Printing, a turnstile relay, and the screen stay in the host.

The queue that runs today lives in `src/registration_server/sync.rs`. The registration server is the first host. The library moves to `modules/event-sync` when a second host links it. The wire paths stay the ones in [login-and-token.md](../registration-server/login-and-token.md). The queue rules stay the ones in [sync-implementation-plan.md](../registration-server/sync-implementation-plan.md).

## What the library owns

- One queue. Its length is the socket cap the host compiled (`SYNC_IN_FLIGHT`, default 100).
- Downloads of one path stay in order. The next call of that path is appended after the body is stored.
- Uploads of different paths share sockets. The next batch of the same upload path is appended after that batch is stored. `boxapi/uploadphoto` calls for different ids may sit in the queue together.
- A free socket takes a waiting download. An upload is sent when no download is waiting.
- Cursors move only after the rows commit.
- A registration created on the device stays off the scan and attachment uploads until `synch_res` accepts its id. A registration downloaded from the server does not hold them. An edit clears `in_synch` and leaves that fact alone.
- `error_msg` of `not_logged_in` or `no_pattern` is reported to the host as a dead token. Any other `error_msg` stops the queue and leaves the binding.

The library does not draw a badge, talk to a turnstile, or serve HTTP to the floor.

## What the host binds

Three pieces. The registration server binds them with `CurrentEvent`, `RemoteServer`, and the full catalog.

| Piece | What the host supplies |
|---|---|
| Transport | Send a GET, a POST form, or a multipart body to a path. Attach `dev_id`, `token`, and `eid`. Return the body bytes, or the auth failure. |
| Store | The event file: cursors, visitors, zones, scans, attachments, and the `on_server` mark. A host that only checks a door may keep a flat allow-list the library fills. |
| Catalog | Which slices below are on for this process. A slice that is off is not queued. |

The host also supplies `device_id`, `event_id`, and `project_token`, and a function the library calls when the token is dead. That function clears the binding and closes the event file. The registration server's clear is that function.

A host that also serves local requests or prints binds the gate from [design.md](../registration-server/design.md). The library waits on that gate before it parses a body. A phone and a door reader have no gate. They call one wake when the radio is up.

## Catalog slices

Each slice is the paths already implemented. The host turns a slice on. The dependency order inside a slice stays.

| Slice | Paths | Stored as |
|---|---|---|
| `reference` | `boxapi/dbenums`, then one `boxapi/dbenum` | enum lists |
| `forms` | `boxapi/getconf`, `boxapi/getbg`, `boxapi/ticketbarcodes` | form config, backgrounds, ticket barcodes |
| `zones` | `boxapi/control_zones` | zones |
| `codes` | `boxapi/barcodesinfo`, `boxapi/barcodes`, `boxapi/getbadge` | pools, badge art |
| `people` | `tracked/UniRegUser`, pages of 100 | visitors, marked already on the server |
| `org` | `boxapi/getmanagers`, `tracked/OrgRegUser` | only when the host has an org id |
| `scans_down` | `tracked/ScanUserRef`, pages of 100 | only when the host has a zone id. Downloaded scans are already synced |
| `registrations_up` | `boxapi/uploadreg` in batches of 20, then `boxapi/uploadphoto` | local registrations, then photos of accepted ids |
| `scans_up` | `boxapi/uploadscans` in batches of 100 | scans whose registration is already on the server |
| `attachments_up` | `leadgenapi/upload` in batches of 20 | attachments whose registration is already on the server |

`scans_up` and `attachments_up` leave out a row tied to a registration `uploadreg` has not accepted. A host that leaves `registrations_up` off only ever uploads scans and attachments of people already in the file from `people` or `scans_down`.

## Hosts

| Host | Slices | What stays outside the library |
|---|---|---|
| Registration desk | all of them | Local HTTP, the operator bind, CUPS, badge drawing. [printing.md](../registration-server/printing.md) |
| Access-control station | `zones`, `people`, `scans_down`, `scans_up` | The reader bus and the relay. A badge read looks up the barcode in the stored people and zones, opens the passage or counts the entrance, and writes a scan the library uploads |
| Hall scanner (Android) | `zones`, `people`, `scans_up`, and `scans_down` for the phone's zone | The camera or the built-in scanner, and the screen that shows who entered and whether that person may be in this hall |
| Door reader | `people` as a short allow-list, `scans_up` | The badge input and the latch |

The access-control station and the hall scanner answer "may this person enter?" from rows the library has stored. The count of entrances is a read of the scans table for that barcode. The library does not keep a second counter.

A door reader with a few kilobytes of RAM speaks two calls and keeps the allow-list in flash: one download of barcode and access, and `boxapi/uploadscans` for the scans it has taken. An ESP32 whose firmware is Rust links the reduced profile and uses the same two slices. Both speak the same paths as the registration desk.

## How a host links it

The crate type is the one `rust-reg` already builds: `rlib` for a Rust host, `cdylib` and `staticlib` for a C ABI. Symbols for the C ABI start with `event_sync_`. Android loads that library through JNI. The phone passes the catalog slices, the event directory, and the token. Java calls the same functions the registration server calls from Rust.

```text
event_sync_open(dir, device_id, event_id, token, slices) -> handle
event_sync_wake(handle)                                  probe, then the queue
event_sync_lookup(handle, barcode) -> allow, name        local file, no socket
event_sync_add_scan(handle, barcode, zone, time)         local file, uploaded on a later wake
event_sync_close(handle)
```

`event_sync_lookup` and `event_sync_add_scan` are the calls a hall scanner and a door reader make between wakes. The registration desk keeps using its own HTTP routes for search and scan, and those routes write the same store.

A dead token makes `event_sync_wake` return the rejected status. The host clears the binding. The library does not delete credential files by itself when it is linked this way. The registration server's host binding does call `CurrentEvent::clear`, which is that host's choice.

## What stays in the registration server

`remote.rs` stays the operator bind: login, the event list, `profile/synchdev`. The gate, the nice-15 sync thread, print, and local HTTP stay in `src/registration_server`. They bind the library. They are not part of it.

The second host is the access-control station. Linking it is the step that moves the queue, the tokened transport, and the catalog slices into `modules/event-sync`. Until that link, the registration server compiles the library in place, and the tests in `registration_server::sync` cover the queue.
