# Porting login, the event file, sync, printing, the local HTTP API, and the registration UI

The device logs in once, stores the project token, opens the database for the selected event, keeps that event in sync, prints badges on the server thread, and answers the local HTTP API the floor and the operator use. The registration screens are a separate React Router app. A version tag publishes the Linux, macOS, and Windows packages, and the install asks before it switches to a newer build. The wire contract is [login-and-token.md](login-and-token.md). Local HTTP stays ahead of remote work as in [design.md](design.md). Inside the sync thread a download is taken before an upload. Drawing and CUPS are [printing.md](printing.md). The local routes are [http-implementation-plan.md](http-implementation-plan.md). The form is [form-design.md](form-design.md).

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
| [8. Printing](#step-8--printing) | Draw a badge and submit it to CUPS | in progress |
| [9. Local HTTP API](#step-9--local-http-api) | The `/api` routes the floor and the desk call | in progress |
| [10. Registration UI](#step-10--registration-ui) | The form, then the operator screens around it | in progress |
| [11. Release and update](#step-11--release-and-update) | GitHub release packages, then a checked swap with rollback | not started |

Step 8's layout, both renderers, and the in-memory badge are done. Queues, routing, and the print worker are not started.

Step 10's form plan is done. Its operator-screen plan is done through the form path (step 5). Visitors, print, import, settings, and printers are not started.

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
- The local HTTP API: registrations, search, scans, forms, printers, and keys. The mustache pages are not served.
- The registration UI: the kiosk at `/register` and the desk at `/desk`. Both run the form from [form-design.md](form-design.md). That app is its own project and can be built before printing and before the local HTTP API.
- The three release packages and the updater that installs a newer GitHub release after the operator agrees and the checksum matches.

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

Status: in progress. The layout, both renderers, and preparing them when the event opens are done. Queues through the worker are not started.

The design is [printing.md](printing.md). The build sequence is [printing-implementation-plan.md](printing-implementation-plan.md).

The server thread finds CUPS queues, draws one badge with the `ticket-render` crate, and submits that job. The sync thread stores `badge.cfg` and `bg.png` from `boxapi/getbadge` and does not draw or print. `ticket-render` is its own crate. It returns PNG bytes or a 1-bit graphic. It does not call cupsd.

- At startup, and when the operator asks, ask the local cupsd over IPP for queues and for USB devices that have no queue. A device with no queue is installed once, with a PPD already on the device.
- `POST /print` names a registration. The worker picks a queue from the event routing (explicit name, else used queues matched by category, internet, and zone, oldest `lastPrint`), renders, and submits one IPP `Print-Job`. One job runs. Further jobs wait on the print channel. The worker yields between pages.
- A Zebra or TSC queue receives ZPL. Every other installed queue receives a PNG and CUPS filters it. The PPD on disk is left as the device image wrote it. Job options (media size, Zebra tracking, Evolis ink) travel on that job.
- A failed submit is an error to the caller. The worker does not write a stand-in PNG and report success.

## Step 9 — Local HTTP API

Status: in progress. Liveness, the form files, visitors, desk, and sync status are done. Print, scans, and leadgen are not started.

The design and the build sequence are [http-implementation-plan.md](http-implementation-plan.md).

This is the last server step. It replaces the Scala `RegApp` routes the floor and the desk call. Responses are JSON on `/api`. The process does not render a mustache page.

- A registration is saved, searched, printed, and scanned through the open event file. Print jobs go to the step 8 worker.
- Forms, enums, backgrounds, zones, and ticket barcodes are the files sync already stored.
- Keys, printer settings, and routing are the operator routes. They require the operator cookie from step 3.
- `GET /health` stays. The skeleton `GET /registrations/{id}`, `POST /print`, and `GET /ws` fold into the routes in that plan.

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

Status: not started.

The build sequence is [release-implementation-plan.md](release-implementation-plan.md). It does not wait on a printer. `print_busy` is a job already on the print channel.

A tag `vX.Y.Z` builds three packages and publishes them on one GitHub release. The running install looks for a newer release. It asks the operator, and it restarts only after the file's SHA-256 matches the release. A start that never opens the listen socket is rolled back to the previous tree. After a start that does open it, that previous tree is a dated backup, and a backup older than seven days is removed.

The event directory and the credential file stay outside the versioned tree. A swap and a rollback leave both where they are.

### Publish

`.github/workflows/release.yml` runs on a tag `release-X.Y.Z`, or from a manual run that types `X.Y.Z` and checks Linux, macOS, and Windows. The workflow writes `X.Y.Z` into `Cargo.toml` and pushes that commit when the version changed. The checked runners build that commit in parallel. Each builds the release binary and the static apps `registration-admin/` and `registration-form/`, and bakes `vX.Y.Z` into that binary. One later job creates or updates the GitHub release `release-X.Y.Z` and uploads the built assets and `SHA256SUMS`. A version with `-alpha`, `-beta`, or `-rc` is marked pre-release. A version without that suffix is not. A manual run that leaves a system unchecked does not rebuild that asset, and the previous file of that name stays on the release.

| Runner | Asset | How it is packed |
|---|---|---|
| `ubuntu-latest` | `rust-reg_<version>_amd64.deb` | `dpkg-deb` |
| `macos-latest` | `rust-reg-<version>.pkg` | `pkgbuild` |
| `windows-latest` | `rust-reg-<version>.exe` | the installer that writes the same tree |

`SHA256SUMS` is one line per asset: the hex digest, two spaces, the file name.

The first install uses the package. A later update extracts the new tree from that same asset and does not run maintainer scripts, `installer`, or the exe's start-the-service path. The Linux unit, the launchd plist, and the Windows service stay the ones the first install registered. Each of them starts the supervisor `bin/rust-reg-run`. That supervisor is the parent of the server and lives outside `versions/`, so a new tree does not replace it.

Which tree is live is one file, `updates/live`, holding a version. The supervisor writes it by renaming a temporary file over it. Version directories are not renamed onto each other. The previous directory stays in place until a start has opened the socket.

```text
<prefix>/
  bin/rust-reg-run                      supervisor
  versions/<version>/
    rust-reg                            rust-reg.exe on Windows
    admin/                              registration-admin dist
    form/                               registration-form dist
    RELEASE                             one line, the tag compiled into rust-reg
  backup/<version>-<YYYYMMDD>/          moved here after a start that opened the socket
  updates/
    live                                one line, the version to exec
    state.yml                           staged, trying, current, or rolled_back
    started                             written by the child after it is listening
```

`<prefix>` is `/opt/rust-reg` on Linux, `/Library/rust-reg` on macOS, and `C:\Program Files\rust-reg` on Windows. `dpkg-deb -x` unpacks the deb into a partial directory that is then renamed to `versions/<version>/`. The pkg payload is expanded the same way, without `installer`. The exe is run with a staging directory and without starting the service, and that directory is renamed to `versions/<version>/` the same way.

### Look, then ask

A newer release is read from GitHub, `GET https://api.github.com/repos/<owner>/<repo>/releases/latest`. The remote registration server has no update path, and this check does not call it. The sync thread only schedules that request, once the listen socket is open and again every six hours. It is not a sync download. The project token is not sent. A draft or a prerelease is skipped. `tag_name` is kept when it is `v` plus a version greater than the release tag compiled into the binary. A version is `MAJOR.MINOR.PATCH`, or those numbers plus `-alpha.N`, `-beta.N`, or `-rc.N`. `REGISTRATION_UPDATE_REPO` overrides the compiled `owner/repo`.

`GET /api/update` requires the desk cookie. The data is the running version and, when the check found one, the newer version. It does not call GitHub. `POST /api/update/check` requires the same cookie and runs that GitHub lookup immediately, then returns the same data. A failure leaves the previous offer and returns `update_check_failed`. The desk menu shows "Обновить" with that version. Confirming posts `POST /api/update`. Until that post, nothing is downloaded and the process does not exit. `/register` does not show the control. The menu test `menu_omits_update_and_moderation` gains the control for a waiting update in the same change; the moderation checkbox stays absent. The menu does not post `POST /api/update/check`.

The Scala paths `GET /updatecheck`, `GET /version`, `GET /update`, and `GET /need_update` stay unserved.

### Install

`POST /api/update` requires the desk cookie. A print job still in the channel returns `print_busy` and leaves the process running.

1. Download this machine's asset next to `versions/` under a temporary name. The download waits out the same gate as sync: local HTTP and a print job go first.
2. Download `SHA256SUMS` from that release and compare the digest of the asset. A mismatch deletes the file and returns `checksum_mismatch`. `updates/live` stays the running version.
3. Unpack into `versions/<new>.partial`, then rename that directory to `versions/<new>/`. The rename is the moment the tree is complete.
4. Write `state.yml` with phase `staged`, `from` the running version, and `to` the new version. Delete `updates/started` if it remains. Fsync the state file. `updates/live` still names `from`.
5. Exit. The supervisor points `live` at the new tree.

On its next start the supervisor reads `state.yml`:

- Phase `staged`: write `updates/live` as `to`, fsync that file, set the phase to `trying`, fsync `state.yml`, then exec `versions/<to>`. The phase is durable before the exec, so a child that dies is the rollback case below.
- Phase `trying` with no `started` file: the last exec never opened the socket. Write `updates/live` back to `from`, delete `versions/<to>/`, set the phase to `rolled_back`, and exec `versions/<from>`. `versions/<from>/` was never moved. A later check can offer a newer release. This failed version is not tried again.
- Phase `trying` with a `started` file: the socket opened, and the child exited before it finished the backup move. Finish that move, set the phase to `current`, exec `versions/<to>`.
- Phase `current` or `rolled_back`: exec the version in `updates/live`.

### After the socket is open

The new process writes `updates/started` and fsyncs it once the listen socket is open. While the phase is `trying`, it then moves `versions/<from>/` to `backup/<from>-<YYYYMMDD>/`, sets the phase to `current`, and fsyncs `state.yml`. It removes each backup directory whose date is more than seven days before today.

A crash after `started` is on disk is a normal restart of the version in `updates/live`. The supervisor leaves that version in place. The backup from that start is what remains of the old build until the seven days pass.
