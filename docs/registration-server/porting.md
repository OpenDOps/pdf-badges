# Porting login, the event file, sync, printing, the local HTTP API, and the registration UI

The device logs in once, stores the project token, opens the database for the selected event, keeps that event in sync, prints badges on the server thread, and answers the local HTTP API the floor and the operator use. The registration screens are a separate React Router app. A release tag publishes the Linux, Apple silicon, Intel macOS, and Windows packages, and the install asks before it switches to a newer build. Each package also carries a tray wrapper that starts that server. The wire contract is [login-and-token.md](login-and-token.md). Local HTTP stays ahead of remote work as in [design.md](design.md). Inside the sync thread a download is taken before an upload. Drawing and CUPS are [printing.md](printing.md). The local routes are [http-implementation-plan.md](http-implementation-plan.md). The form is [form-design.md](form-design.md). A Google Sheet can replace the remote sync queue. That choice is step 13.

The remote API stays. The Scala session scraper, the properties-file side effects, and the sync loop tangled into "save the token" do not.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Credential file](#step-1--credential-file) | `device_id`, the base URL, and the project token on disk | done |
| [2. Remote client](#step-2--remote-client-for-the-bind) | Login, the event list, and `profile/synchdev` | done |
| [3. Local bind](#step-3--local-way-to-run-the-bind) | The admin app: login, choose an event, store the token | done |
| [4. Event database](#step-4--event-database) | The event file and the memory copy | done |
| [5. Switch](#step-5--switch-load-or-init) | Open, init, or load the selected event | done |
| [6. Tokens](#step-6--working-with-tokens) | Send the stored token and drop a dead binding | done |
| [7. Sync](#step-7--sync) | The download and upload queue | done |
| [8. Printing](#step-8--printing) | Draw a badge and submit it to CUPS | done |
| [9. Local HTTP API](#step-9--local-http-api) | The `/api` routes the floor and the desk call | done |
| [10. Registration UI](#step-10--registration-ui) | The form, then the operator screens around it | in progress |
| [11. Release and update](#step-11--release-and-update) | GitHub release packages, then a checked swap with rollback | in progress |
| [12. Tray wrapper](#step-12--tray-wrapper) | Open the tray on Linux, macOS, and Windows | not started |
| [13. Google Sheet source](#step-13--google-sheet-source) | Admin binds a sheet; the sync thread applies its row events | not started |

Step 8 is done. Steps 1 through 19 of the printing plan are done.

Step 9 is done. Steps 1 through 6 of the HTTP plan are done. The scan API and the leadgen API are not ported.

Step 10's form plan is done. Its operator-screen plan is done through the form path (step 5). Visitors, print, import, settings, and printers are not started.

Step 11 is checked on macOS. The Linux deb and the Windows exe have been published and have not been installed or updated on those machines. The step is accepted when that manual test has passed on both, and when an older binary has read a release from `api.github.com`.

Step 12's tray code is in `desk/`. The icon has not been opened on Linux, macOS, or Windows. `cargo test --manifest-path desk/Cargo.toml` does not open a display. The step is accepted when the checks in [tray-wrapper-testing-plan.md](tray-wrapper-testing-plan.md) have passed on all three.

Step 13 is not started. The watcher it consumes is [google-tables/implementation-plan.md](../google-tables/implementation-plan.md). This step is the admin choice, the local processes, and the sync-thread consumer.

The stored id is `event_id`. The remote paths stay `boxapi/expos`, `eid`, and `profile/synchdev/{device_id}/{event_id}`.

## What this plan owns

- A provisioned `device_id` and a configured base URL.
- The four remote calls that turn an email and password into a project token.
- A credential file that survives restart.
- The event SQLite file and the in-memory working set for search and the registration list. [db-design.md](db-design.md) is that pair.
- Switching to the selected event: load its file, or init the tables when the file is new. Both close when the binding is cleared or switched.
- One HTTP helper that attaches `dev_id`, `token`, and `eid` to a request, proves a stored token, and drops the binding when the server says it is dead.
- A sync queue of ready downloads and uploads. Local HTTP runs first, then a download, then an upload. The queue is as long as the socket cap.
- Printing a badge on the server thread: find a CUPS queue, draw it with the ticket-render crate, send one IPP job. The sync queue does not print.
- The local HTTP API: registrations, search, forms, printers, and keys. The mustache pages are not served. The scan API and the leadgen API are not ported.
- The registration UI: the kiosk at `/register` and the desk at `/desk`. Both run the form from [form-design.md](form-design.md). That app is its own project and can be built before printing and before the local HTTP API.
- The release installers, the upgrade zips, and the updater that installs a newer GitHub release after the operator agrees and the checksum matches.
- The tray wrapper, `rust-reg-desk`, built once for Linux, macOS, and Windows. It starts the server and shows the listen address. The registration screens stay in the browser.
- A sheet source on the same device: the admin binds a Google Sheet, and the sync thread applies that watcher's row events instead of the remote download queue. [sheets-source.md](sheets-source.md) is that split. Building the watcher is the google-tables plan, not this step.

## What this plan leaves

- `tracked/UsersPay` and `user/log/actions`. The Scala tree does not send them. `getScansData` returns without a request. `UpdateChecker.check` is empty in that tree; step 11 talks to GitHub instead.

## Step 1 — Credential file

Status: done.

The file library is [credentials.md](credentials.md). The build sequence is [credentials-implementation-plan.md](credentials-implementation-plan.md). `CredentialFile` lives in `src/registration_server/credentials.rs`.

One file, loaded at process start, written when the bind succeeds or the token is cleared.

```text
device_id          required, written by provisioning, not by login
base_url           https://kuprin.su/ or the stage host
event_id            empty until an event is chosen
event_name          display label, optional
project_token      empty until profile/synchdev returns it
```

Read `curentr_expo_token` if a file from the Scala client is opened, and write the new key on the next save.

Empty strings are not a logged-out state. Logout deletes `event_id`, `event_name`, and `project_token`.

If `event_id` and `project_token` are both present at start, the process is already logged in. It does not call `auth/login`. Opening that event's file is step 5.

## Step 2 — Remote client for the bind

Status: done.

`Remote` and `OperatorSession` are in `src/registration_server/remote.rs`. `reqwest` performs the calls. The `cookie` crate parses `JSESSIONID` from `Set-Cookie`, and later calls send that cookie. The temptoken is not stored. `bind` writes the project token through `CredentialFile` and drops the session, so the cookie dies with it.

At process start the server reads `REGISTRATION_REMOTE`. Unset or empty means `kuprin.su`, used as `https://kuprin.su/`. A host without a scheme is `https`. An `http` URL is rewritten to `https`. Loopback is left as `http` so the local stand-in can answer. The resolved base is logged as `registration-server remote ...` and kept on `Config.remote_base`.

The automated tests answer a local stand-in. They do not call kuprin.su and do not need an account there.

A separate check talks to the real server. `cargo test` skips it. Run it by hand and pass the account as flags:

```text
cargo test --test live_remote -- --email you@example.com --password secret --device-id device-1
```

`--remote` overrides `REGISTRATION_REMOTE`. That command logs in and lists events. It does not bind an event. Adding `--event EXPO_ID --out base/credentials.yml` calls `profile/synchdev` and writes the project token. That attaches `device-id` to that event on the server.

Four functions, each one HTTP call. Parse JSON. Keep the `JSESSIONID` only in memory, and drop it when the project token is stored.

1. `POST auth/login` with `email` and `pass`. Read `temptoken`. A body with no token and a failing status is `invalid_cred`. A connect failure or a timeout is `no_connection`. Missing `device_id` fails before the request, as `no_device_id`.
2. `GET /` with `ak-fivesec-token` and `ak-fivesec-token-email`. Read `JSESSIONID` from `Set-Cookie`.
3. `GET boxapi/expos/ru` with `limit`, `skip`, and the cookie. Return `uniqueId` and `name.ru.str` for each entry in `list`. `You are not logged in!` means the cookie died; the operator logs in again.
4. `GET profile/synchdev/{device_id}/{event_id}` with the cookie. Store `token` from the JSON. Then discard the cookie.

The temptoken is exchanged in the same login action. It is not saved.

## Step 3 — Local way to run the bind

Status: done.

The operator uses a React admin app: log in, pick an event, and the device stores the project token. Visible copy comes from a Russian catalog through `react-i18next`. The build sequence is [admin-implementation-plan.md](admin-implementation-plan.md).

The Scala pages (`admin.mustache`, `controlExpo.mustache`, jQuery, Angular) stay in the old tree. This step does not render them.

Vite writes a static build. The registration server serves that directory. A conditional GET returns a file when it changed and `304 Not Modified` when it did not, using `ETag`, `Last-Modified`, `If-None-Match`, and `If-Modified-Since`.

The browser talks JSON to `/api` on the HTTP server. That is not the floor gRPC service (`Lookup`, `EnqueuePrint`). Login stays `POST` and the event list stays `GET`: the password is sent once, and the event page reloads the list with the cookie. Success is `{ "ok": true, "data": ... }`. Failure is `{ "ok": false, "error": { "code": "..." } }` with the status in the [admin plan](admin-implementation-plan.md#api). An HttpOnly cookie names the operator session. That cookie is not the remote `JSESSIONID`. The operator lease lasts one minute. The events page posts `/api/session` every 30 seconds, and each post sets the deadline to one minute from then.

| action | local route | remote call |
|---|---|---|
| submit email and password | `POST /api/login` | calls 1 and 2, hold the session for this operator |
| list events | `GET /api/events` | call 3 |
| choose one event | `POST /api/events/select` | call 4, persist the project token; the operator cookie stays |

Choosing an event replaces the previous `event_id` and `project_token`. Sync cursors for the new event start empty. Opening that event's file is step 5.

The event screen has a keys block. `GET /api/keys` returns an empty list until the key routes in the admin plan read the table step 4 creates.

## Step 4 — Event database

Status: done.

The event file and the in-memory working set. [db-design.md](db-design.md) is the schema, the Scala `prepareDatabase` path, and which queries stay off the flash. The build sequence is [db-implementation-plan.md](db-implementation-plan.md).

`rusqlite` opens `base/{event_id}/db.sqlite` and a shared `mode=memory` database. The file is the durable copy. Search and the registration list read the memory database. WAL is on for the file and `synchronous` is `FULL`. The server thread and the sync thread each open their own connection to each database and do not share a connection.

The file tables from the Scala event file are created empty when the file is new. The memory database gets the FTS index and the list-page rows. `keys` gets one admin row and one operator row when it is empty. No remote payload is parsed. Which event is open, and when the memory copy runs, is step 5.

## Step 5 — Switch, load, or init

Status: done.

When the operator chooses an event, and again at startup when the credential file already names one, switch to that event's file. The build sequence is [switch-implementation-plan.md](switch-implementation-plan.md).

`POST /api/events/select` writes the binding, then switches. Startup does the same open when `event_id` and `project_token` are both present. It does not call `auth/login`.

- If `base/{event_id}/db.sqlite` is missing, or an event table is missing, init the step 4 schema. A new `keys` table gets the two default rows. `index_state` is `ready` when the file has no visitors.
- If the file is already there, open it and load the memory copy in the background, in the table `last_add` order. The first commit is the first 100 rows, the first five pages. Search uses those committed rows while `index_state` is `loading`. The list page reads the file until the copy is `ready`, then reads memory.
- The previous event's file and memory database close after the next event is open. Rows are not copied from the previous file.
- Clearing the binding closes both.

A switch clears the sync queue for the previous event and drops calls still running for that id. The new event is probed on the next wake, and that wake fills its own queue after step 6 accepts the token.

## Step 6 — Working with tokens

Status: done.

The build sequence is [token-implementation-plan.md](token-implementation-plan.md). That client is its own module. `remote.rs` stays the operator bind: login, the event list, and `profile/synchdev`.

Every later remote call, including each call the sync queue sends, goes through the remote-server client:

```text
request(method, path, fields) ->
    require device_id, event_id, project_token
    send fields plus dev_id, token, eid
```

POST form is the default. GET puts the same fields in the query (`boxapi/barcodesinfo`). Multipart adds files and keeps the fields (`boxapi/uploadphoto`). A missing credential returns the local error `invalid_cred` and does not open a socket. The client does not know what each path means. Callers pass their own fields.

After a bind, and again on startup once step 5 has the event file open, send one tokened request and read only the auth outcome. `GET boxapi/barcodesinfo` has no extra fields. HTTP 200 with a JSON body and no `error_msg` of `not_logged_in` or `no_pattern` means the token is accepted. The `info` array waits for the barcode download in step 7.

Do this before the queue sends anything else. The sync thread is the place that repeats it. The login action itself stays on the server thread as one operator request.

If a tokened response has `error_msg` `not_logged_in` or `no_pattern`, delete the project binding, close the event file and the memory database, and stop scheduling remote calls until the operator binds again. Any other `error_msg` keeps the token. The queue is cleared.

`invalid_cred` stays local. It means the file has no token. It does not write empty properties.

## Step 7 — Sync

Status: done.

The build sequence is [sync-implementation-plan.md](sync-implementation-plan.md). The queue is the `event-sync` library. This process is the first host and turns on every slice. An access-control station, a phone scanning a hall, and a door reader bind the same library with a smaller catalog. [event-sync/design.md](../event-sync/design.md) is that split. Printing and the local HTTP API stay in this process.

The sync thread keeps one queue, filled on the same 60-second timer as `Synchronizer.startSynch`. A tick that finds the queue busy does not fill it again. Every call goes through step 6. A dead token clears the queue. An event switch drops calls still running for the previous id and clears that queue.

Downloads and uploads are queued together. The queue is as long as the socket cap (`SYNC_IN_FLIGHT`, default 100). Downloads of the same path stay in order. Different paths share the sockets. An upload is sent only when the local file has rows waiting and no download is waiting for a socket. An empty outbox does not open a socket.

Three ranks, highest first:

1. Local HTTP, and a print job on the server thread. [design.md](design.md) is that split: the server thread at nice 0, the sync thread at nice 15, and the gate closed while a local request or a print job is running. The next remote read-and-parse waits out that gate. Scala pauses the same way: `getPermission` sleeps while `permissionToWork` is false.
2. A download. When a remote slot is free and a download is waiting, the download takes it.
3. An upload. It runs in the gaps where no download is waiting for a slot.

At most the compiled `SYNC_IN_FLIGHT` remote calls are in flight (default 100, the cap in [design.md](design.md)). `--sync-in-flight` replaces it for one process. Bodies are streamed. The process parses one body at a time.

Downloads follow the live Scala order. Each call sends its cursor (`last_synch`, `from_time`, `version`, or `clean_id`) and writes the payload into the event file from step 4, then the memory rows step 5 is loading.

| order | path | when |
|---|---|---|
| 1 | `boxapi/dbenums`, then `boxapi/dbenum` | reference lists. `dbenum` follows each dependency |
| 2 | `boxapi/getconf`, `boxapi/getbg` | form config and backgrounds. Once with an empty form id, then once per form id. `getbg` uses `resolution` `pad` and `phone` |
| 3 | `boxapi/ticketbarcodes` | once with an empty form id, then once per form id |
| 4 | `boxapi/control_zones` | zones |
| 5 | `boxapi/barcodesinfo` | the probe in step 6 already hits this path. The queue reads `info` here |
| 6 | `boxapi/getbadge` | badge art, per category, and again for certificates |
| 7 | `boxapi/barcodes` | barcode pools. `cat_id`, `take`, `bc_type`, and either `cert_id` or `multi_day=true` |
| 8 | `tracked/UniRegUser` | visitors, pages of 100, `from_time` |
| 9 | `boxapi/getmanagers` | only when an org id is set |
| 10 | `tracked/ScanUserRef` | only when a zone id is set. Pages of 100 |
| 11 | `tracked/OrgRegUser` | only when an org id is set. Pages of 100 |

Scala calls `tracked/OrgRegUser` after the uploads. This queue keeps it on the download side, so it is taken before an upload.

The table is the dependency list. Calls of the same download path run one at a time. Different paths are queued together.

Uploads follow the Scala order. A batch that the server accepts is marked synced, and the pipeline asks for the next batch.

| order | path | when |
|---|---|---|
| 1 | `boxapi/uploadreg` | local registrations waiting to go up, 20 at a time. `vals` is the JSON text |
| 2 | `boxapi/uploadphoto` | multipart, for photos of the ids `uploadreg` just accepted |
| 3 | `boxapi/uploadscans` | scans waiting to go up, 100 at a time. A scan of a new registration waits until `uploadreg` accepts that registration |
| 4 | `leadgenapi/upload` | attachments waiting to go up, 20 at a time. `no_need_data=true` and `expoId`. An attachment of a new registration waits until `uploadreg` accepts that registration |

## Step 8 — Printing

Status: done. Steps 1 through 19 of [printing-implementation-plan.md](printing-implementation-plan.md) are done.

The design is [printing.md](printing.md). The build sequence is [printing-implementation-plan.md](printing-implementation-plan.md).

The server thread finds CUPS queues, draws one badge with the `ticket-render` crate, and submits that job. The sync thread stores `badge.cfg` and `bg.png` from `boxapi/getbadge` and does not draw or print. `ticket-render` is its own crate. It returns PNG bytes or a 1-bit graphic. It does not call cupsd.

- At startup, and when the operator asks, ask the local cupsd over IPP for queues and for USB devices that have no queue. A device with no queue is installed once, with a PPD already on the device.
- `POST /print` names a registration. The worker picks a queue from the event routing (explicit name, else used queues matched by category, internet, and zone, oldest `lastPrint`), renders, and submits one IPP `Print-Job`. One job runs. Further jobs wait on the print channel. The worker yields between pages.
- A Zebra or TSC queue receives ZPL. Every other installed queue receives a PNG and CUPS filters it. The PPD on disk is left as the device image wrote it. Job options (media size, Zebra tracking, Evolis ink) travel on that job.
- A failed submit is an error to the caller. The worker does not write a stand-in PNG and report success.

## Step 9 — Local HTTP API

Status: done. Steps 1 through 6 of [http-implementation-plan.md](http-implementation-plan.md) are done: liveness, the form files, visitors, print, desk, and sync status. The scan API and the leadgen API are not ported.

The design and the build sequence are [http-implementation-plan.md](http-implementation-plan.md).

This is the last server step. It replaces the Scala `RegApp` routes the floor and the desk call. Responses are JSON on `/api`. The process does not render a mustache page.

- A registration is saved, searched, and printed through the open event file. Print jobs go to the step 8 worker.
- Forms, enums, and backgrounds are the files sync already stored.
- Keys, printer settings, and routing are the operator routes. They require the operator cookie from step 3.
- `GET /health` stays. `GET /ws` stays the idle lookup socket. The skeleton `GET /registrations/{id}` and `POST /print` fold into the routes in that plan.

## Step 10 — Registration UI

Status: in progress. The form plan is done. The operator screens are done through the form path. Visitors onward are not started.

The form is [form-design.md](form-design.md), and its build sequence is [form-implementation-plan.md](form-implementation-plan.md). The other operator screens are [registration-ui-design.md](registration-ui-design.md), and their build sequence is [registration-ui-implementation-plan.md](registration-ui-implementation-plan.md).

This is a second React project, `registration-form/`. It does not live inside `registration-admin/`. Login and the event choice stay that admin app. This app is the registration form and the operator desk around it: the key, the menu, the form, the visitor list, print progress, import, settings, and printers.

Navigation is React Router in framework mode (the continuation of Remix). `/register` is the kiosk. `/form` is the operator form and renders the desk component already written for `/desk`. A form step is engine state. It is not a URL segment, because the next screen is chosen by the config's conditions and Back pops that stack. Every other operator screen is its own route module, fetched when a link to it is on the screen the operator is viewing.

The form reads one document, `GET /forms/form.json`. Vite builds that response from the unchanged source files in `registration-form/public/forms/`: vars, structure, the empty model, the quest config, settings, and barcodes, with `jv_rights` omitted. Country, region, and city lists stay in `registration-form/public/dbenums/`. Tests fetch those URLs. They do not start the registration server, so this step can land before step 8 and before step 9. When those steps exist, the same client calls `/api`.

- `build` walks the config into questions and the first step. `setValue` writes the visitor. Next checks the step, then follows `next`.
- Country, region, city, and the phones on one address are that address object. The mask comes from the selected country row.
- `/register` serves the kiosk layout, or the phone layout when `Sec-CH-UA-Mobile` is `?1`. Inside the kiosk layout, `ipad` and `web` follow the viewport. `/form` uses the operator frame and the same step component. Save posts the visitor. Idle reset runs only on `/register`.

## Step 11 — Release and update

Status: in progress. The macOS pack and the updater tests are done. The Linux and Windows install and update are step 16 of the release plan, and they have not been run. The live check against `api.github.com` has not been run. The step is accepted only after both.

The build sequence is [release-implementation-plan.md](release-implementation-plan.md). It does not wait on a printer. `print_busy` is a job already on the print channel, or a print the worker has started.

A tag `release-X.Y.Z` builds the installers and an upgrade zip for each system, and publishes them on one GitHub release. The binary bakes `vX.Y.Z`. The running install looks for a newer release. It asks the operator, and it restarts only after the zip's SHA-256 matches the release. A start that never opens the listen socket is rolled back to the previous tree, and that version is not offered again. After a start that does open it, that previous tree is a dated backup. A backup dated seven or more days earlier is removed.

The credential file and the event directory stay in `data/`. A swap, a rollback, and a backup move leave both where they are.

### Publish

`.github/workflows/release.yml` runs on a tag `release-X.Y.Z`, or from a manual run that types `X.Y.Z` and checks Linux, macOS, and Windows. The workflow writes `X.Y.Z` into `Cargo.toml` and `Cargo.lock` and pushes that commit when the version changed. One Ubuntu job builds `registration-admin` and `registration-form` once. The checked runners then build that commit in parallel, reuse those files, and bake `vX.Y.Z` into `rust-reg` and `rust-reg-run`. Each runner first tests `registration_server::native_print` for its own system: Linux and macOS compile the CUPS client, and Windows compiles the spooler client. Checking macOS builds both Apple silicon and Intel. One later job creates or updates the GitHub release `release-X.Y.Z` and uploads the built assets and `SHA256SUMS`. A version with `-alpha`, `-beta`, or `-rc` is marked pre-release. A version without that suffix is not. A manual run that leaves a system unchecked does not rebuild that asset, and the previous file of that name stays on the release.

| Runner | Installer | Upgrade zip |
|---|---|---|
| `ubuntu-latest` | `rust-reg_<version>_amd64.deb` | `rust-reg-<version>-linux.zip` |
| `macos-latest` | `rust-reg-<version>.pkg` | `rust-reg-<version>-macos.zip` |
| `macos-15-intel` | `rust-reg-<version>-intel.pkg` | `rust-reg-<version>-macos-intel.zip` |
| `windows-latest` | `rust-reg-<version>.exe` | `rust-reg-<version>-windows.zip` |

The deb is `dpkg-deb`. Each pkg is `pkgbuild`. The exe is the server binary with a ustar of the install tree appended. A local pack follows the CPU: Apple silicon uses `stable-aarch64-apple-darwin`, and an Intel Mac uses `stable-x86_64-apple-darwin`. `SHA256SUMS` is one line per installer and one line per zip: lowercase hex, two spaces, the file name.

The first install uses the package. It copies `versions/<version>/rust-reg-run` to `bin/rust-reg-run` and registers the service. A later update downloads the zip. It does not run maintainer scripts, `installer`, or the exe's start-the-service path, and it does not rewrite the unit, the launchd plist, or the Windows service. Each of them starts `bin/rust-reg-run`. That supervisor is the parent of the server. It starts `versions/<live>/rust-reg registration-server` and passes `--credentials <prefix>/data/credentials.yml`, `--dist <prefix>/versions/<live>/admin`, and `--form <prefix>/versions/<live>/form`.

Which tree is live is one file, `updates/live`, holding a version. `updates/live` and `state.yml` are written as a temporary file in the same directory, then fsync, rename, and fsync the directory. Version directories are not renamed onto each other.

```text
<prefix>/
  bin/rust-reg-run                      first install; an update replaces this file in place
  bin/rust-reg-desk                     tray; first install; an update leaves this file
  versions/<version>/
    rust-reg                            rust-reg.exe on Windows
    rust-reg-run                        rust-reg-run.exe on Windows; same tag as rust-reg
    admin/                              registration-admin dist
    form/                               registration-form build
    RELEASE                             one line, the baked tag
  backup/<version>-<YYYYMMDD>/
  updates/
    live                                one line, the version to start
    state.yml                           phase, from, to, rejected
    started                             written after the listen socket is open
  data/credentials.yml
  data/<event_id>/
```

`<prefix>` is `/opt/rust-reg` on Linux, `/Library/rust-reg` on macOS, and `C:\Program Files\rust-reg` on Windows. `state.yml` `phase` is `staged`, `trying`, `current`, or `rolled_back`. `rejected` lists versions that failed before the socket opened. When the server that staged the upgrade was started from outside `versions/<from>/`, `state.yml` also records that executable and, when they are not already the installed trees, the `admin` and `form` directories it was serving.

### Look, then ask

A newer release is read from GitHub, `GET https://api.github.com/repos/OpenDOps/pdf-badges/releases/latest`. The remote registration server has no update path, and this check does not call it. The sync thread only schedules that request, once the listen socket is open and again every six hours. It is not a sync download. The request sends `User-Agent: rust-reg` and `Accept: application/vnd.github+json`. The project token is not sent, and there is no `Authorization` header. A draft or a prerelease is skipped. `tag_name` is kept when it is `v` or `release-` plus a version greater than the tag compiled into the binary, and that version is not in `rejected`. A version is `MAJOR.MINOR.PATCH`, or those numbers plus `-alpha.N`, `-beta.N`, or `-rc.N`. For the same numbers, `alpha` is before `beta`, `beta` is before `rc`, and a version with no suffix is newer than one with a suffix. `REGISTRATION_UPDATE_REPO` overrides `OpenDOps/pdf-badges`. `REGISTRATION_UPDATE_PRERELEASE=1` instead reads `GET /repos/<owner>/<repo>/releases` and keeps pre-releases, then offers the newest of those versions. A failed request leaves the previous offer.

`GET /api/update` requires the desk cookie. The data is the running version and, when the check found one, the newer version. It does not call GitHub. `POST /api/update/check` requires the same cookie and runs that lookup immediately, then returns the same data. A failure leaves the previous offer and returns `502` `update_check_failed`. A check already in flight is `429` `check_in_progress`. A missing cookie is `401` `unauthorized`. The desk menu shows "Обновить" with that version. Confirming posts `POST /api/update`. Until that post, nothing is downloaded and the process does not exit. `/register` does not show the control. The moderation checkbox stays absent. The menu does not post `POST /api/update/check`.

The Scala paths `GET /updatecheck`, `GET /version`, `GET /update`, and `GET /need_update` stay unserved.

Two checks are still open. An older binary, started against a scratch prefix, reads `https://api.github.com`, and `GET /api/update` names that binary's version and the newer release. Separately, the published deb is installed and then updated on a Linux machine, and the published exe is installed and then updated on a Windows machine. A macOS pass does not accept those two packages.

### Install

`POST /api/update` requires the desk cookie. A job on the print channel, or the gate's print flag, returns `409` `print_busy` and leaves the process running.

1. Download this machine's upgrade zip next to `versions/` as `<name>.partial`. The file is `rust-reg-<version>-linux.zip`, `rust-reg-<version>-macos.zip` on Apple silicon, `rust-reg-<version>-macos-intel.zip` on Intel, or `rust-reg-<version>-windows.zip`. macOS reads `hw.optional.arm64`, so an Intel build running under Rosetta on Apple silicon still takes the Apple silicon zip. The download waits until the gate has no print in progress and no local request other than this one.
2. Download `SHA256SUMS` from that release and compare the digest. A mismatch deletes the file and returns `409` `checksum_mismatch`. `updates/live` stays the running version.
3. Unpack into `versions/<new>.partial`, fsync, then rename that directory to `versions/<new>/`. The payload is `rust-reg` (`rust-reg.exe` on Windows), `rust-reg-run`, `admin/`, `form/`, and `RELEASE`. A ustar of those same entries is accepted too. The rename is the moment the tree is complete. A failed copy removes `.partial`. `bin/rust-reg-run` is left as it is.
4. Write `state.yml` with phase `staged`, `from` the running version, and `to` the new version. Delete `updates/started` if it remains. `updates/live` still names `from`.
5. Return `200` and `{ "version": "<to>" }`, then exit. The supervisor points `live` at the new tree.

On its next start the supervisor reads `state.yml`:

- Phase `staged`: when `versions/<from>/` is missing or empty and `updates/live` names another version that has a tree, `from` becomes that version. An executable recorded in `running` that sits outside `versions/<from>/` is copied into `backup/<from>-<YYYYMMDD>/`, with the `admin` and `form` trees, and the source stays. Write `updates/live` as `to`, remove `updates/started`, set the phase to `trying`, then start `versions/<to>`. The phase is durable before that start.
- Phase `trying` with no `started` file: the new process never opened the socket, or it could not be spawned. Write `updates/live` back to `from`. Restore a missing `versions/<from>/rust-reg` from the newest `backup/<from>-*` directory, and restore a missing `admin/` or `form/` when the binary is already there. Delete `versions/<to>/`, append `to` to `rejected`, set the phase to `rolled_back`, and start `versions/<from>`. A later check skips a version in `rejected`.
- Phase `trying` with a `started` file: the socket opened, and the child exited before it finished the backup move. Finish that move, set the phase to `current`, and start `versions/<to>`.
- Phase `current` or `rolled_back`: start the version in `updates/live`.

A stop of the service stops the child and the parent exits. The phase and the version directories stay.

### After the socket is open

The new process writes `updates/started` and fsyncs it once the listen socket is open, before the sync thread's first update check. While the phase is `trying`, it moves the previous tree to `backup/<version>-<YYYYMMDD>/`, sets the phase to `current`, and fsyncs `state.yml`. The previous tree is `versions/<from>/` when that directory has files. When it is missing or empty, the tree is the one `updates/live` names, if that version is not `to` and that directory has files. When neither directory has a tree, no backup is created and the phase still becomes `current`. A backup dated seven or more days earlier is removed. A later start with phase `current` does not move `versions/` again and does not rewrite `started`.

Once the phase is `current`, the running wrapper is compared with `versions/<to>/rust-reg-run`. The wrapper reads its own path. The server reads the parent executable when that file is named `rust-reg-run` or `rust-reg-run.exe`. The same digest leaves the file. A different digest replaces it, fsyncs the new file, and fsyncs the directory that holds it. A missing path does not create `bin/rust-reg-run`. The process that performed the switch keeps the image it was started with. The next start of that same path is the new wrapper. A phase that is still `staged`, `trying`, or `rolled_back` does not copy. A failed copy leaves the previous file, leaves `versions/<to>/` in place, and leaves the phase `current`.

`data/credentials.yml` and `data/<event_id>/` stay in place across the unpack, the rollback, the backup move, and the wrapper copy.

## Step 12 — Tray wrapper

Status: not started.

This step is the manual check. The tray code is already in `desk/`. The check is [tray-wrapper-testing-plan.md](tray-wrapper-testing-plan.md). `cargo test --manifest-path desk/Cargo.toml` covers the listen line, which binary an installed path starts, and the macOS icon pixels. It does not open a display. A pass on one machine does not cover the other two.

The server process has no window. `rust-reg-desk` starts it and shows a tray icon: a status icon on Linux, a menu-bar item on macOS, and a notification-area icon on Windows. The menu shows the listen address and Quit. A second launch exits. Quit stops the server this tray started, or the installed service when that service already held the port.

## Step 13 — Google Sheet source

Status: not started.

The design is [sheets-source.md](sheets-source.md). The watcher is [google-tables/design.md](../google-tables/design.md), and its build sequence is [google-tables/implementation-plan.md](../google-tables/implementation-plan.md). This step starts when that watcher can publish a `RowChange` to local JetStream. It does not reimplement header finding or the sheet scan.

One event has one source, `remote` or `sheet`. `remote` is steps 2 through 7: the project token and the download and upload queue. `sheet` replaces that queue for the open event. The floor HTTP API and printing stay as they are. Visitor uids are the UIDs google-tables wrote into the sheet.

The credential file gains `sync_source`. Empty means `remote`, so a file written by steps 1 through 7 stays a remote binding. `sheet` stores the spreadsheet id, the Google `sheetId`, the excluded fingerprint columns, and the map from sheet column name to visitor field. The event id is the spreadsheet id. `project_token` stays empty. The service-account JSON is a file next to the credentials, not a field in the YAML. Logout deletes the sheet fields the same way it deletes `event_id` and `project_token`.

The admin event screen offers the source. `remote` keeps `POST /api/login`, `GET /api/events`, and `POST /api/events/select`. `sheet` uses:

| action | local route | what it calls |
|---|---|---|
| choose the sheet source | `POST /api/source` | stores `sync_source` `sheet` and starts the local watcher |
| open a spreadsheet | `POST /api/sheet/open` | google-tables `OpenSpreadsheet` |
| pick the tab and the header, or name the columns | `POST /api/sheet/header` | `SelectSheet`, then `SelectHeader` or `SelectColumns`, then `SetColumnOptions` |
| bind column names to visitor fields | `POST /api/sheet/bind` | stores the map, calls `EnsureIdentity` and `StartWatch`, then step 5 opens `base/{spreadsheet_id}` |

`POST /api/source` with `remote` stops the watcher and the local JetStream consumer and returns the operator to the event list. A running remote queue is cleared first, the same way step 5 clears a queue on an event switch.

The release zip carries `google-tables` and `nats-server` beside `rust-reg`. Both are static binaries. `nats-server` listens on `127.0.0.1` only. The subject is `sheets.<watch_id>`. `rust-reg` starts them when `sync_source` is `sheet`, including on startup when the credential file already says so, and stops them when the source changes or the binding is cleared. The tray still starts only `rust-reg`.

The sync thread subscribes as the durable consumer `registration-server`. It does not call kuprin and does not fill the download or upload queue. Local HTTP and printing still pass the gate in [design.md](design.md) before a batch is applied.

| event | event file |
|---|---|
| `SNAPSHOT` | replace the visitor rows, in batches of 100 |
| `ADDED` | insert that uid |
| `UPDATED` | replace that uid |
| `DELETED` | remove that uid |
| `MAPPING` | store the new column list and apply no further row events until `POST /api/sheet/bind` covers every name that already had a binding |

A name with no binding is kept on the stored row and is not a visitor column. The consumer resumes from its JetStream cursor after a restart. The sheet snapshot stays in the google-tables LMDB file.
