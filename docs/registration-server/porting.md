# Porting login and token reuse

First slice of the registration server: log in once, store the project token, and send that token on later remote calls. The wire contract is [login-and-token.md](login-and-token.md). Scheduling of downloads against local HTTP and print stays as in [design.md](design.md). This slice does not pull registration rows, render badges, or print.

The remote API stays. The Scala session scraper, the properties-file side effects, and the sync loop tangled into "save the token" do not.

## What this slice owns

- A provisioned `device_id` and a configured base URL.
- The four remote calls that turn an email and password into a project token.
- A credential file that survives restart.
- One HTTP helper that attaches `dev_id`, `token`, and `eid` to a request.
- One existing tokened call used as a probe, so a stored token is known to work before any payload is parsed.
- Clearing the stored token when the server says the bind is dead.

## What this slice leaves

- Parsing and storing `getconf`, enums, visitors, barcodes, badges, scans, uploads. The path list in the login document is the catalog for those slices.
- Operator keys, key printing, and the per-exhibition sqlite file. The React admin screen reserves that block. Those routes start in the step after this one. Saving the token only saves the token and records that a project was chosen.

## Step 1 — Credential file

The file library is [credentials.md](credentials.md). The build sequence is [credentials-implementation-plan.md](credentials-implementation-plan.md). `CredentialFile` lives in `src/registration_server/credentials.rs`.

One file, loaded at process start, written when the bind succeeds or the token is cleared.

```text
device_id          required, written by provisioning, not by login
base_url           https://kuprin.su/ or the stage host
expo_id            empty until a project is chosen
expo_name          display label, optional
project_token      empty until profile/synchdev returns it
```

Read `curentr_expo_token` if a file from the Scala client is opened, and write the new key on the next save.

Empty strings are not a logged-out state. Logout deletes `expo_id`, `expo_name`, and `project_token`.

If `expo_id` and `project_token` are both present at start, the process is already logged in. It does not call `auth/login`.

## Step 2 — Remote client for the bind

`Remote` and `OperatorSession` are in `src/registration_server/remote.rs`. `reqwest` performs the calls. The `cookie` crate parses `JSESSIONID` from `Set-Cookie`, and later calls send that cookie. The temptoken is not stored. `bind` writes the project token through `CredentialFile` and drops the session, so the cookie dies with it.

At process start the server reads `REGISTRATION_REMOTE`. Unset or empty means `kuprin.su`, used as `https://kuprin.su/`. A host without a scheme is `https`. An `http` URL is rewritten to `https`. Loopback is left as `http` so the local stand-in can answer. The resolved base is logged as `registration-server remote ...` and kept on `Config.remote_base`.

The automated tests answer a local stand-in. They do not call kuprin.su and do not need an account there.

A separate check talks to the real server. `cargo test` skips it. Run it by hand and pass the account as flags:

```text
cargo test --test live_remote -- --email you@example.com --password secret --device-id device-1
```

`--remote` overrides `REGISTRATION_REMOTE`. That command logs in and lists exhibitions. It does not bind a project. Adding `--expo EXPO_ID --out base/credentials.json` calls `profile/synchdev` and writes the project token. That attaches `device-id` to that exhibition on the server.

Four functions, each one HTTP call. Parse JSON. Keep the `JSESSIONID` only in memory, and drop it when the project token is stored.

1. `POST auth/login` with `email` and `pass`. Read `temptoken`. A body with no token and a failing status is `invalid_cred`. A connect failure or a timeout is `no_connection`. Missing `device_id` fails before the request, as `no_device_id`.
2. `GET /` with `ak-fivesec-token` and `ak-fivesec-token-email`. Read `JSESSIONID` from `Set-Cookie`.
3. `GET boxapi/expos/ru` with `limit`, `skip`, and the cookie. Return `uniqueId` and `name.ru.str` for each entry in `list`. `You are not logged in!` means the cookie died; the operator logs in again.
4. `GET profile/synchdev/{device_id}/{expo_id}` with the cookie. Store `token` from the JSON. Then discard the cookie.

The temptoken is exchanged in the same login action. It is not saved.

## Step 3 — Local way to run the bind

The operator uses a React admin app: log in, pick an exhibition, and the device stores the project token. Visible copy comes from a Russian catalog through `react-i18next`. The build sequence is [admin-implementation-plan.md](admin-implementation-plan.md).

The Scala pages (`admin.mustache`, `controlExpo.mustache`, jQuery, Angular) stay in the old tree. This step does not render them.

Vite writes a static build. The registration server serves that directory. A conditional GET returns a file when it changed and `304 Not Modified` when it did not, using `ETag`, `Last-Modified`, `If-None-Match`, and `If-Modified-Since`.

The browser talks JSON to `/api` on the HTTP server. That is not the floor gRPC service (`Lookup`, `EnqueuePrint`). Login stays `POST` and the exhibition list stays `GET`: the password is sent once, and the exhibition page reloads the list with the cookie. Success is `{ "ok": true, "data": ... }`. Failure is `{ "ok": false, "error": { "code": "..." } }` with the status in the [admin plan](admin-implementation-plan.md#api). An HttpOnly cookie names the operator session. That cookie is not the remote `JSESSIONID`. The remote cookie stays inside `OperatorSession` until bind.

| action | local route | remote call |
|---|---|---|
| submit email and password | `POST /api/login` | steps 1 and 2, hold the session for this operator |
| list exhibitions | `GET /api/exhibitions` | step 3 |
| choose one exhibition | `POST /api/exhibitions/select` | step 4, persist, forget the remote cookie |

Choosing a project replaces the previous `expo_id` and `project_token`. Sync cursors for the new project start empty when those slices exist. This slice only swaps the credential.

The exhibition screen has a keys block. `GET /api/keys` returns an empty list. Writing keys is the next step.

## Next — exhibition sqlite and keys

After step 3, and before the token is attached to the sync round. Selecting an exhibition switches the open sqlite file to that exhibition's file. Admin and operator keys live in that file. The React screen then fills that block from `GET /api/keys`, `POST /api/keys`, `PATCH /api/keys/:id`, `DELETE /api/keys/:id`, and `POST /api/keys/print`. Step 3 does not create the database.

## Step 4 — Token on every later call

One function used by the sync thread:

```text
request(method, path, fields) ->
    require device_id, expo_id, project_token
    send fields plus dev_id, token, eid
```

POST form is the default. GET puts the same fields in the query (`boxapi/barcodesinfo`). Multipart adds files and keeps the fields (`boxapi/uploadphoto`, later). A missing credential returns a local error and does not open a socket.

The helper does not know what each path means. Callers pass their own fields. That is the whole reuse mechanism.

## Step 5 — Prove the stored token

After a bind, and again on startup when a token is already on disk, send one tokened request and read only the auth outcome.

`GET boxapi/barcodesinfo` has no extra fields. HTTP 200 with a JSON body and no `error_msg` of `not_logged_in` or `no_pattern` means the token is accepted. The `info` array can be ignored until the barcode slice.

Do this before scheduling the rest of the round in [design.md](design.md). The sync thread is the place that repeats it: the login action itself is rare and can run on the server thread as one operator request.

## Step 6 — Drop a dead token

If a tokened response has `error_msg` `not_logged_in` or `no_pattern`, delete the project binding and stop scheduling remote calls until the operator binds again. Any other `error_msg` keeps the token.

`invalid_cred` stays local. It means the file has no token. It does not write empty properties.

## Later slices

Each row is one remote path from the catalog, built on step 4. Order follows the Scala round, which is also a workable build order: configuration before visitors, downloads before uploads.

1. `boxapi/dbenums` and `boxapi/dbenum` — reference lists.
2. `boxapi/getconf` and `boxapi/getbg` — form config and backgrounds, per form id.
3. `boxapi/control_zones`, `boxapi/getbadge` — zones and badge art.
4. `boxapi/barcodesinfo`, `boxapi/barcodes`, `boxapi/ticketbarcodes` — barcode pools. The probe in step 5 already hits `barcodesinfo`.
5. `tracked/UniRegUser` — visitor download.
6. `boxapi/uploadreg`, `boxapi/uploadphoto`, `boxapi/uploadscans`, `leadgenapi/upload` — local changes back up.
7. `boxapi/getmanagers`, `tracked/OrgRegUser`, `tracked/ScanUserRef` — only when an org id or a zone id is configured.

`tracked/UsersPay` and `user/log/actions` stay out until a caller needs them. The Scala tree does not send them.
