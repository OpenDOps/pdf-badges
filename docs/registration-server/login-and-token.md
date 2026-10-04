# Login and the project token

The venue device logs in once, binds itself to one event, and keeps the token that bind returns. Every later call to the remote server sends that token. The password and the browser session are only for the bind.

The Scala client is `temp-reg/registration`. The remote host is `http://kuprin.su/`, or `http://stage.kuprin.su/` when `devmode` is set. `getHostStr(secure = true)` uses the same `http://` URLs. This document is the wire contract to follow. [porting.md](porting.md) is the order to build it. [design.md](design.md) is how the process shares one core once sync is running.

## Two credentials

```text
email + password
        │
        ▼
POST auth/login                         short-lived temptoken
        │
        ▼
GET  /  ?ak-fivesec-token=…             Set-Cookie: JSESSIONID
        │
        ▼
GET  boxapi/expos/ru                    list of events (cookie)
        │
        ▼
GET  profile/synchdev/{device}/{event}   project token (cookie)
        │
        ▼
stored: device_id + event id + token
        │
        ▼
every sync call                         form fields dev_id, token, eid
```

`JSESSIONID` authorizes the list and the bind. After the project token is stored, the client drops the session. Restart does not log in again: startup reads the stored token and starts the sync timer.

## What must already be on the device

`device_id` is a string in `./base/main.props`. The application reads it and never writes it. Login refuses with `no_device_id` when the key is missing. Provision the id with the machine.

The remote base URL is configuration. The Scala client hardcodes the two hosts above. The Rust process reads `REGISTRATION_REMOTE` at start. Unset, that is `https://kuprin.su/`. An `http` URL is rewritten to `https`.

## Calls that produce the token

Form bodies are `application/x-www-form-urlencoded`. The Scala client scrapes some of these bodies as raw text. The port parses JSON.

### 1. Password to temptoken

`POST {host}auth/login`

| field | value |
|---|---|
| `email` | operator email |
| `pass` | password |

HTTP 200. The body contains `"temptoken":"<value>"`. The client finds the substring `temptoken`, skips the three characters `":"`, and reads until the next quote.

If `temptoken` is absent, the client looks for `status` and reads one digit eight characters later, which is the digit in `"status":N`. A digit greater than 0 is `invalid_cred`. A body that is not that shape throws `NumberFormatException`, which the login page reports as `no_connection`.

The query name on the next call, `ak-fivesec-token`, is the lifetime: the temptoken exists to be exchanged immediately.

### 2. Temptoken to session cookie

`GET {host}`

| query | value |
|---|---|
| `ak-fivesec-token` | the temptoken |
| `ak-fivesec-token-email` | the same email |

HTTP 200 and a `Set-Cookie` header. The client takes the text between the first `=` and the first `;`. That value is `JSESSIONID`.

The local admin page then keeps `loggedin` and `jsessionid` on its own HTTP session and redirects to `/controlExpo`. That session is the operator sitting at the device. It is not the sync credential.

### 3. List events

`GET {host}boxapi/expos/ru`

| query | value |
|---|---|
| `limit` | default `1000` (`expos_list_limit`) |
| `skip` | page offset, default `0` |

Cookie: `JSESSIONID=<session>`.

HTTP 200 JSON:

- `list` — events. The page reads `uniqueId` and `name.ru.str`.
- `status.errors[0].msg` — when present. The string `You are not logged in!` means the cookie is dead.

The local route `GET /exposlist?jsessionid=&offset=` is a proxy. It adds `current` (the stored event id) next to `list` before rendering.

### 4. Bind this device, take the project token

`GET {host}profile/synchdev/{deviceId}/{expoUID}`

Cookie: `JSESSIONID=<session>`. No query parameters.

HTTP 200 JSON. The only field the client stores is `token`.

On that field the client writes `./base/main.props` and drops the operator session:

| property key | meaning |
|---|---|
| `current_expo_uid` | event id (`uniqueId`) |
| `curentr_expo_token` | project token. The key is misspelled in the file. |
| `current_expo_name` | label from the page (`name.ru.str`), not from this response |

`setCurrentExpoUID` also zeroes sync cursors, opens the local event database, and starts the 60-second sync loop. Those are consequences of choosing a project. The token itself is the three properties above. [db-design.md](db-design.md) is that database open, including the working connection the Scala client keeps beside the file.

Local route: `POST /selectexpo` with `expoUID`, `jsessionid`, and optional `expoName`. It requires the local `loggedin` session, calls the URL above, stores the token, then `removeSession`.

## Stored state the sync loop needs

On startup the process loads `./base/main.props` (Java `Properties`). If `current_expo_uid` is non-empty it opens local event data. If both the uid and `curentr_expo_token` are present it starts the synchronizer. No password, no cookie.

Each write copies the file to `./base/main.props.back` first and restores that copy when the main file is empty.

`cleanCurrentExpoUIDToken` deletes `current_expo_uid`, `curentr_expo_token`, and `current_expo_name`.

The Rust process keeps these values in its own file. [credentials.md](credentials.md) is that file. It reads `curentr_expo_token` from a Scala properties file and writes `project_token`.

## How a sync call carries the token

One helper builds every authenticated sync request. If `device_id`, the event id, or the token is missing, it does not call the server and returns the local error `invalid_cred`.

Otherwise it adds three fields to the endpoint's own fields:

| field | source |
|---|---|
| `dev_id` | `device_id` |
| `token` | project token |
| `eid` | event id |

Four transports, same three fields:

| helper | transport |
|---|---|
| `tokenedRequestStr` | POST form, response body as text |
| `tokenedRequestStrGet` | GET query, response body as text |
| `tokenedRequestIs` | POST form, response kept as a stream (zip or octet-stream) |
| `tokenedRequestFUpload` | POST multipart: files plus the form fields |

The sync loop in `Synchronizer.runSyncTask` runs these in order, on one thread, every 60 seconds. A round still running is skipped. Payloads and cursors are later slices. The paths below are the surface that must accept the three fields.

Live, in round order:

| path | method | extra fields |
|---|---|---|
| `boxapi/dbenums` | POST | `enums`, `version`, `langs` |
| `boxapi/getconf` | POST | `last_synch`, `last_cnfids_synch`, `conf_id` (empty, then once per form id) |
| `boxapi/getbg` | POST | `last_synch`, `resolution` (`pad`, `phone`), `conf_id` |
| `boxapi/ticketbarcodes` | POST | `conf_id`, `clean_id` |
| `boxapi/control_zones` | POST | `last_synch` |
| `boxapi/barcodesinfo` | GET | none |
| `boxapi/getbadge` | POST | `last_synch`, `cat_id`, `is_cert` |
| `boxapi/barcodes` | POST | `cat_id`, `take`, `bc_type`, and either `cert_id` or `multi_day=true` |
| `tracked/UniRegUser` | POST | `from_time`, `json` (`{eid}_reg_track_request`), `limit` |
| `boxapi/getmanagers` | POST | `last_synch`, `orgid` — only when `org_id` is set |
| `tracked/ScanUserRef` | POST | `from_time`, `json` (`{eid}_regscan_track_request_short`), `extraId` (`zone_{zoneId}`), `limit` |
| `boxapi/uploadreg` | POST | `vals` (JSON text) |
| `boxapi/uploadphoto` | POST multipart | files named from local uid photos, fields `f_{id}` |
| `boxapi/uploadscans` | POST | `vals` |
| `leadgenapi/upload` | POST | `vals`, `no_need_data=true`, `expoId` |
| `tracked/OrgRegUser` | POST | `from_time`, `json`, `limit`, `extraId` (org id) — only when `org_id` is set |
| `boxapi/dbenum` | POST | `enum`, `langs`, `gzipped`, plus dependency ids |

`boxapi/getconf`, `boxapi/control_zones`, `boxapi/getmanagers`, and `boxapi/dbenums` are the calls that clear the stored token on `not_logged_in` or `no_pattern`. The others throw or log.

Dead in the current tree, so the port can ignore them until something still calls them:

- `tracked/UsersPay` — the payment download and the scan download that reused this path are commented out. `getScansData` returns without a request.
- `user/log/actions` — inside a block comment in `Persistence.scala`.
- `UpdateChecker.check` — empty. It does not send the token.

## When the server rejects the token

JSON field `error_msg`:

| value | client reaction |
|---|---|
| `not_logged_in` | delete uid, token, and name; stop the sync loop |
| `no_pattern` | same delete and stop |
| anything else | log, keep the token, continue the round |

`invalid_cred` is local: the three fields were missing, so no HTTP call was made. Some callers then run `setCurrentExpoUID("", "")`, which writes empty strings and does not delete the keys. A later read still sees a value. Logout is `cleanCurrentExpoUIDToken`, which removes the keys.

## Operator-facing errors on the login page

`POST /admin` with `login` and `password` is the local page. It maps failures to:

| `error_msg` | cause |
|---|---|
| `no_device_id` | `device_id` missing, remote not called |
| `invalid_cred` | login body had no temptoken and a positive status digit |
| `no_connection` | connect failure, timeout, or a body the scraper could not parse |
| `unknown_error` | any other exception |

The "remember me" checkbox and the "forgot password" link on `admin.mustache` have no server handler.
