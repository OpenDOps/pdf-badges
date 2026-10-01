# Admin pages — step 3

Build sequence for [porting.md](porting.md) step 3. The operator logs in, sees the exhibition list, chooses one, and the credential file from [credentials.md](credentials.md) stores the project token. The remote calls are the ones already in `Remote` and `OperatorSession` ([login-and-token.md](login-and-token.md)).

The admin UI is a React application. Copy lives in a Russian catalog and reaches the screen through `react-i18next`. The Scala templates (`admin.mustache`, `controlExpo.mustache`) and their jQuery, Angular, and Bootstrap assets stay in the old tree.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 3 is done when steps 1 through 12 are done. Keys and the per-exhibition sqlite file are the [following plan](#following-plan--exhibition-sqlite-and-keys), not a step here.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. React package](#step-1-react-package) | Vite, TypeScript, Vitest, `react-i18next` | done |
| [2. Russian catalog](#step-2-russian-catalog) | Every visible string, keyed | done |
| [3. Login screen](#step-3-login-screen) | Form, submit, error sentence | done |
| [4. Exhibition screen](#step-4-exhibition-screen) | List, choose, token line, empty keys | done |
| [5. Conditional GET](#step-5-conditional-get) | `ETag`, `Last-Modified`, `304` for one file | done |
| [6. Public path](#step-6-public-path) | URL path stays inside the build directory | done |
| [7. Static route](#step-7-static-route) | Those rules on the HTTP router | done |
| [8. Login failures](#step-8-login-failures) | `POST /api/login` returns an error code | not started |
| [9. Login success](#step-9-login-success) | Cookie set, exhibition route can load | not started |
| [10. Exhibition list](#step-10-exhibition-list) | `GET /api/exhibitions` | not started |
| [11. Choose exhibition](#step-11-choose-exhibition) | Select stores the token; keys stay empty | not started |
| [12. Mounted server](#step-12-mounted-server) | Built assets and `/api` on `serve` | not started |

Shared rules:

- The UI lives in `registration-admin/`. Rust routes live in `src/registration_server/admin.rs` and join the HTTP router in `src/registration_server/http.rs`. The server thread already holds the gate for a request, so login does too.
- The same stack as the PDF editor: React, TypeScript, Vite. Localization is `i18next` and `react-i18next`. The locale is `ru`. There is no second language in this step.
- Components call `t("key")`. A missing key fails the Vitest run. The server sends an error code. The catalog maps `error.<code>` to a sentence. The server does not send Russian prose.
- The operator cookie is HttpOnly. It names the in-memory `OperatorSession`. It is not the remote `JSESSIONID`. The remote cookie stays inside that session until `bind` drops it.
- Tests that talk to a remote use a local stand-in with the same four routes as `remote.rs`: `POST auth/login`, `GET /`, `GET boxapi/expos/ru`, `GET profile/synchdev/{device}/{expo}`. They do not call kuprin.su.
- `POST /api/login` accepts JSON `login` and `password`. Map them to `email` and `pass` on `Remote::login`.

## Screens

| screen | route | what it shows |
|---|---|---|
| Login | `/` | Email, password, submit. A failed login shows the sentence for the error code. |
| Exhibitions | `/exhibitions` | The list, the current exhibition, a line that the project token is stored or absent, and an empty keys block. |

The token line says that a token is stored. It does not print the token.

```text
registration-admin/
  package.json
  index.html
  src/
    main.tsx
    i18n.ts
    locales/ru.json
    api.ts
    Login.tsx
    Exhibitions.tsx
    App.tsx
  dist/                 vite build, served by the registration server
```

`vite build` writes `dist/`. The registration server serves that directory as static files. `index.html` is the response for `/` and `/exhibitions`. A path under `/api` is never replaced with `index.html`.

## API

The admin page and `/api` share the HTTP server. The gRPC service on the other port stays the floor API: `Lookup` and `EnqueuePrint`. It does not gain login or exhibition methods.

A browser page cannot call that service without a second client stack, and the operator id is already an HttpOnly cookie on the same origin as the page. The password, the remote `JSESSIONID`, and the project token are never response fields. `Remote` still calls `POST auth/login` and `GET boxapi/expos/ru` on the remote host. The browser does not.

Login and the exhibition list stay separate calls.

- `POST /api/login` is the command that checks the password and mints the cookie. A GET would put the password in the URL, and a reload of the exhibition page must not send it again.
- `GET /api/exhibitions` reads that session's list. Repeating it does not log in.
- `POST /api/exhibitions/select` writes the credential file.
- `GET /api/binding` reads the file after select has cleared the cookie, so the screen can still show the chosen exhibition.

### JSON

`Content-Type` is `application/json`. Success is `{ "ok": true, "data": <value> }`. Failure is `{ "ok": false, "error": { "code": "<code>" } }`. There is no message field. The screen uses `t("error." + code)`.

`ok` follows the status class: `2xx` is `true`. Any other status this API returns is `false` and carries a code.

| code | status | when |
|---|---|---|
| `bad_request` | 400 | The JSON is missing, or a required field is empty. |
| `no_device_id` | 400 | The credential file has no device id. No socket is opened. |
| `invalid_cred` | 401 | The remote rejected the password. |
| `session_expired` | 401 | The cookie is missing or unknown, or the remote session died. |
| `login_in_progress` | 429 | A login is already inside `Remote::login`. The extra call is not queued and does not open `auth/login`. |
| `no_connection` | 502 | Connect failed, the call timed out, or the login body was not the remote shape. |
| `unknown_error` | 500 | Any other remote failure, including a remote HTTP error, a bad URL, a credential-file error, and `DeviceMismatch`. |
| `not_found` | 404 | No such `/api` route. The static handler does not answer these. |
| `keys_later` | 501 | A key write, until the sqlite plan. |

### Routes

| route | `data` |
|---|---|
| `POST /api/login` | Request `{ "login", "password" }`, mapped to remote `email` and `pass`. Success is `200`, `data` `{}`, and `Set-Cookie` HttpOnly, `SameSite=Lax`, `Path=/`. |
| `GET /api/exhibitions` | Cookie required. `{ "current": { "id", "name" } or null, "exhibitions": [ { "id", "name" } ] }`. `current` comes from the credential file. |
| `POST /api/exhibitions/select` | Cookie required. Body `{ "id", "name" }`. An empty `id` is `bad_request`. Success is `200` and `{ "id", "name", "token_stored": true }`. The cookie is cleared. |
| `GET /api/binding` | No cookie. `{ "id", "name", "token_stored" }` from the credential file. `id` and `name` are `null` when unset. `token_stored` is `true` only when `project_token` is set. |
| `GET /api/keys` | `[]`. |
| `POST /api/keys`, `POST /api/keys/print` | No `data`. Status `501`, code `keys_later`. |

## Static response

The static handler serves `registration-admin/dist` at the URL root. `GET` and `HEAD` only.

Validators are the ordinary conditional-GET headers. `Cache-Control: no-cache` tells the client to revalidate on every use. The handler then either sends the bytes or says the cached copy is still good.

On `200` and on `304`:

| header | value |
|---|---|
| `ETag` | strong tag, `"<mtime seconds>-<length>"` |
| `Last-Modified` | HTTP-date of the file mtime |
| `Cache-Control` | `no-cache` |

`200` also sets `Content-Type` from the extension and `Content-Length`. `HEAD` uses the same headers and an empty body.

Request checks, in this order:

1. `If-None-Match` lists the current `ETag`, or `*`. Answer `304` with an empty body. A list that does not contain the current tag is `200`, including when `If-Modified-Since` would have matched.
2. Otherwise `If-Modified-Since` is at or after `Last-Modified` (HTTP-date is whole seconds). Answer `304`.
3. Otherwise `200` and the file bytes.

A rebuild that changes the length or the mtime changes the `ETag`, so the next GET is `200`. `/api` responses do not send these validators.

## Catalog

`src/locales/ru.json` holds every string the two screens show. The keys:

```text
login.title
login.email
login.password
login.submit
error.bad_request
error.no_device_id
error.invalid_cred
error.no_connection
error.unknown_error
error.session_expired
error.login_in_progress
exhibitions.title
exhibitions.choose
exhibitions.current
exhibitions.empty
exhibitions.token_stored
exhibitions.token_empty
keys.title
keys.empty
```

The Russian values for the four login errors are the sentences in `admin_ru.mustloc` (`Нет соединения с Nexpo.me`, `Неверные имя пользователя или пароль`, `Неизвестная ошибка`, `У устройства нет серийного номера`). The other keys are the Russian labels those pages used as literal HTML: `Вход в систему`, `Введите свой email`, `Введите свой пароль`, `Войти`, and the exhibition and key labels from `controlExpo.mustache` (`Текущая выставка`, `Выберите выставку`, `Ключ администратора` grouped as one keys title).

`error.bad_request` is `Некорректный запрос`. Step 8 adds that key to `ru.json`. `not_found` and `keys_later` stay off the catalog until a screen displays them.

## Step 1. React package

[Back to summary](#summary)

Done. `registration-admin` is a Vite React TypeScript package. `npm test` renders the heading. `npm run build` writes `dist/`.

Create the frontend package. One screen that renders a heading is enough to prove the toolchain.

### Work

1. `registration-admin/package.json` with React, TypeScript, Vite, Vitest, Testing Library, `i18next`, and `react-i18next`.
2. `src/main.tsx` mounts `App`. `App` renders one heading.
3. `npm test` runs Vitest. `npm run build` writes `dist/`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `app_renders` | The heading is in the document. |

### Done when

`npm test` and `npm run build` pass in `registration-admin/`.

## Step 2. Russian catalog

[Back to summary](#summary)

Done. `src/locales/ru.json` is the `ru` catalog. `src/i18n.ts` loads it with `lng` and `fallbackLng` set to `ru`. A missing key throws.

Load `ru.json` through `react-i18next`. Language is `ru`. A missing key is a failed test, not a raw key on screen.

### Work

1. Add `src/locales/ru.json` with every key in [Catalog](#catalog).
2. `src/i18n.ts` initializes i18next with that catalog, `lng: "ru"`, `fallbackLng: "ru"`, and `saveMissing: false`.
3. `main.tsx` imports `i18n.ts` before rendering.

### Test scenarios

| Scenario | Assert |
|---|---|
| `ru_has_every_key` | Each key in the catalog list resolves to a non-empty Russian string. `error.invalid_cred` is `Неверные имя пользователя или пароль`. `login.submit` is `Войти`. |
| `missing_key_fails` | `t("login.missing")` throws, or the test wrapper treats a missing key as a failure. The return value is not the string `login.missing`. |

### Done when

`npm test` passes, and step 1 stays green.

## Step 3. Login screen

[Back to summary](#summary)

Done. `Login.tsx` renders the Russian form. `onSubmit` receives `login` and `password`. While that call's promise is pending, another submit does not call it again.

The login form. It does not call the network yet. The test passes an `onSubmit` and an error code.

### Work

1. `Login.tsx` renders title, email field, password field, and submit, all through `t`.
2. Submit calls `onSubmit(login, password)`.
3. Prop `error` is one of the error codes or empty. The sentence comes from `error.<code>`.
4. A ref records that `onSubmit` is in flight. A second submit returns before calling `onSubmit`. The button is disabled until the promise settles. This is the browser side of one login at a time. The server side is the `429` on `POST /api/login` in step 8.

### Test scenarios

Render with the real catalog.

| Scenario | Assert |
|---|---|
| `login_shows_russian` | Placeholder `Введите свой email`, button `Войти`, title `Вход в систему`. |
| `login_submits_fields` | Typing an email and a password and submitting calls `onSubmit` with those two strings. |
| `login_shows_invalid_cred` | `error="invalid_cred"` shows `Неверные имя пользователя или пароль`. |
| `login_hides_error_when_empty` | `error=""` leaves the error node empty. |
| `login_ignores_submit_while_pending` | `onSubmit` returns a promise that stays pending. A second submit does not call it. The button is disabled. |

### Done when

`npm test` passes, and steps 1–2 stay green.

## Step 4. Exhibition screen

[Back to summary](#summary)

Done. `Exhibitions.tsx` lists names, calls `onSelect` with id and name, and shows the token sentence without the token. The keys block is the title and the empty sentence.

The exhibition list, the token line, and the empty keys block. Data arrives as props.

### Work

1. `Exhibitions.tsx` takes `exhibitions`, `current`, `tokenStored`, and `onSelect(id, name)`.
2. Each row shows `name`. Choosing it calls `onSelect`.
3. `tokenStored` true uses `exhibitions.token_stored`. False uses `exhibitions.token_empty`.
4. The keys block uses `keys.title` and `keys.empty`. It has no add or delete control in this step.

### Test scenarios

| Scenario | Assert |
|---|---|
| `exhibitions_lists_names` | Two exhibitions render both names. The current name sits under `Текущая выставка`. |
| `exhibitions_selects_one` | Activating a row calls `onSelect` with that id and name. |
| `token_stored_line` | `tokenStored` true shows the stored sentence. False shows the empty sentence. The token value is absent. |
| `keys_block_empty` | The keys title is present and the empty sentence is present. |

### Done when

`npm test` passes, and steps 1–3 stay green.

## Step 5. Conditional GET

[Back to summary](#summary)

Decide `200` or `304` for one file. No socket. The rules are [Static response](#static-response).

### Work

1. `fn etag(mtime_secs, len) -> String` returns the quoted strong tag, in `admin.rs`.
2. `fn conditional_get(meta, if_none_match, if_modified_since) -> Freshness` where `Freshness` is `Full` or `NotModified`.
3. Compare `ETag` by exact match against one or more tags in `If-None-Match`, or `*`.
4. Compare `If-Modified-Since` only when `If-None-Match` is absent. HTTP-date resolution is one second.

### Test scenarios

Build the meta from a temp file so mtime and length are real.

| Scenario | Assert |
|---|---|
| `etag_is_mtime_and_length` | A file of 4 bytes has `ETag` `"<mtime>-4"`, including the quotes. |
| `if_none_match_is_304` | The current tag, and `*`, are `NotModified`. |
| `other_tag_is_200` | A different tag is `Full` even when `If-Modified-Since` equals `Last-Modified`. |
| `if_modified_since_is_304` | With no `If-None-Match`, a date equal to `Last-Modified` is `NotModified`. One second earlier is `Full`. |
| `changed_file_is_200` | After the bytes and mtime change, the old tag is `Full` and the new tag differs. |

### Done when

`cargo test --lib registration_server::admin::tests::conditional` passes. `npm test` stays green.

Done. `etag` and `conditional_get` live in `src/registration_server/admin.rs`. A matching `If-None-Match` (one tag, several tags, or `*`) is `NotModified`. A different tag stays `Full` even when `If-Modified-Since` would be fresh. Without `If-None-Match`, an HTTP-date at or after the file's mtime second is `NotModified`.

## Step 6. Public path

[Back to summary](#summary)

Turn a URL path into a file under the build directory, or refuse it.

### Work

1. `safe_public_file(root, url_path) -> Option<PathBuf>`.
2. Decode the path. Reject an empty path, `/`, a trailing slash, `\`, a segment that is `..` or `.`, and a decoded `%2e%2e`.
3. The joined path must be a file whose canonical path is inside the canonical `root`. A directory is `None`. A missing file is `None`.

`/` and `/exhibitions` are not this function's job. Step 12 serves `index.html` for those two routes.

### Test scenarios

Use a temp directory with `assets/app.js` present. A file `secret.txt` sits next to the root, outside it.

| Scenario | Assert |
|---|---|
| `asset_resolves` | `/assets/app.js` is that file. |
| `parent_segment_refused` | `/assets/../secret.txt` and `/assets/%2e%2e/secret.txt` are `None`. |
| `backslash_and_root_refused` | `/assets\\app.js` and `/` are `None`. |
| `missing_file_refused` | `/assets/missing.js` is `None`. |
| `directory_refused` | `/assets` is `None`. |

### Done when

`cargo test --lib registration_server::admin::tests::public_path` passes, and step 5 stays green.

Done. `safe_public_file` percent-decodes the URL path and returns the file only when every segment is a real name and the canonical path is a file inside the canonical root. `..`, `%2e%2e`, `\`, `/`, a trailing slash, a directory, and a missing file are `None`.

## Step 7. Static route

[Back to summary](#summary)

Serve the build directory on the admin router. `/api` is not on this route yet.

### Work

1. `admin_router(root) -> Router` handles `GET` and `HEAD` for a safe public file.
2. Apply steps 5 and 6. Set `Content-Type` from the extension: `.html` `text/html`, `.js` `text/javascript`, `.css` `text/css`, `.svg` `image/svg+xml`, anything else `application/octet-stream`.
3. `304` and `HEAD` have an empty body and still send `ETag`, `Last-Modified`, and `Cache-Control: no-cache`.
4. A refused path and a missing file are `404` with an empty body and without an `ETag`.

Tests call the router with `tower::ServiceExt::oneshot`. Add the `tower` `util` feature as a dev-dependency if the crate does not already expose `oneshot`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `static_get_200` | `GET /assets/app.js` is `200`, the file bytes, `Content-Type: text/javascript`, `Content-Length`, and the three validator headers. |
| `static_revalidate_304` | `If-None-Match` of that `ETag` is `304` and an empty body. The three validator headers are still present. |
| `static_head` | `HEAD` of the same path is `200`, `Content-Length` of the file, and an empty body. |
| `static_escape_404` | `GET /assets/../secret.txt` is `404` and has no `ETag`. |
| `static_post_405` | `POST /assets/app.js` is `405`. |

### Done when

`cargo test --lib registration_server::admin::tests::static_route` passes, and steps 5–6 stay green.

Done. `admin_router` serves `GET` and `HEAD` through steps 5 and 6. `304` and `HEAD` send an empty body with `ETag`, `Last-Modified`, and `Cache-Control: no-cache`. `HEAD` also sends `Content-Length`. A refused or missing path is `404` with an empty body and no `ETag`. Any other method is `405`.

## Step 8. Login failures

[Back to summary](#summary)

`POST /api/login` runs `Remote::login` and returns an error code. The React screen from step 3 maps that code to a sentence. This step tests the server.

### Work

1. App state holds `CredentialFile`, `Remote`, and a session map `local id -> OperatorSession`.
2. Parse JSON `login` and `password`.
3. Map `RemoteError` through the [error table](#json). `NoDeviceId`, `InvalidCred`, and `NoConnection` use their own codes. Every other variant is `unknown_error`.
4. A body that is not JSON, or that omits `login` or `password`, is `400` and `bad_request`. Add `error.bad_request` (`Некорректный запрос`) to `ru.json` and to the key list in `i18n.test.ts`.
5. The session map stays empty. No `Set-Cookie`.
6. One `Remote::login` at a time. While it is running, another `POST /api/login` returns `429` and `login_in_progress` and does not call `auth/login`. The extras are not queued. A queue would still open one remote login per click after the first returned.

`no_device_id` never opens a socket: point `Remote` at `http://127.0.0.1:1`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `missing_device_is_no_device_id` | Empty `device_id`. Status `400`, `{ "ok": false, "error": { "code": "no_device_id" } }`. No stand-in was started. |
| `bad_password_is_invalid_cred` | Stand-in login body has no `temptoken` and `"status":1`. Status `401`, code `invalid_cred`. |
| `down_remote_is_no_connection` | Stand-in accepts and then closes. Status `502`, code `no_connection`. |
| `malformed_login_is_bad_request` | A body that is not JSON is `400` and `bad_request`. No stand-in was started. |
| `failed_login_sets_no_cookie` | Those responses have no `Set-Cookie`. `GET /api/exhibitions` is `401` and `session_expired`. |
| `login_in_progress_skips_remote` | The first `POST /api/login` is still waiting on the stand-in. A second POST is `429` and `login_in_progress`. The stand-in saw one `auth/login`. |

### Done when

`cargo test --test admin_pages login_failure` passes, and steps 5–7 stay green.

## Step 9. Login success

[Back to summary](#summary)

A good login sets the operator cookie.

### Work

1. On `Ok(session)`, mint a local id, store the session, respond `200` and `{ "ok": true, "data": {} }` with `Set-Cookie` HttpOnly, `SameSite=Lax`, `Path=/`.
2. One operator at a time: a second successful login drops the previous session and mints a new id. The old cookie then gets `401`.
3. The stand-in returns a `temptoken` and `Set-Cookie: JSESSIONID=...` on `GET /`, the same contract `Remote::login` already tests.

`src/api.ts` posts `/api/login`. When `ok` is `false`, the login screen shows `t("error." + error.code)`. A Vitest covers that mapping with a mocked `fetch`. The cookie itself is asserted on the Rust side.

### Test scenarios

| Scenario | Assert |
|---|---|
| `login_sets_cookie` | `POST /api/login` against a good stand-in is `200` and `{ "ok": true, "data": {} }`. `Set-Cookie` is HttpOnly. |
| `second_login_replaces_session` | A second good login makes `GET /api/exhibitions` with the first cookie `401`. |
| `login_screen_shows_server_error` | A mocked `fetch` that returns `401` and `{ "ok": false, "error": { "code": "invalid_cred" } }` makes the login screen show the Russian sentence. |

### Done when

`cargo test --test admin_pages login_success` and `npm test` pass, and step 8 stays green.

## Step 10. Exhibition list

[Back to summary](#summary)

`GET /api/exhibitions` returns the list the exhibition screen renders.

### Work

1. The operator cookie selects the session. A missing or unknown cookie is `401` and `session_expired`.
2. Call `OperatorSession::list_exhibitions` with `limit` `1000` and `skip` `0`.
3. `data` is `{ "current": null, "exhibitions": [ { "id": "5245081", "name": "TechCrunch" } ] }`. `id` is `uniqueId`. `name` is `Exhibition.name`. `current` is `{ "id", "name" }` from the credential file, or `null`.
4. `SessionExpired` from the remote removes that local session and returns `401`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `exhibitions_shape` | After login, the stand-in list has one exhibition. `data.exhibitions[0].id`, `data.exhibitions[0].name`, and `data.current: null`. `ok` is `true`. |
| `exhibitions_current_from_file` | A credential file that already has `expo_id` `5245081` and a name makes `data.current` those two fields. |
| `exhibitions_unknown_cookie` | A cookie `nope` is `401` and `session_expired`. |
| `exhibitions_dead_remote_cookie` | The stand-in answers `You are not logged in!`. The response is `401`, and the operator cookie no longer lists exhibitions. |

### Done when

`cargo test --test admin_pages exhibitions` passes, and steps 8–9 stay green.

## Step 11. Choose exhibition

[Back to summary](#summary)

`POST /api/exhibitions/select` binds the device and drops the operator session. The screen can still read the stored binding.

### Work

1. JSON `id` and `name`. An empty `id` is `400` and `bad_request`.
2. Take the session out of the map, then `OperatorSession::bind` with that name.
3. Success is `200` and `data` `{ "id", "name", "token_stored": true }`. The credential file has `expo_id`, `expo_name`, and `project_token`. The cookie is cleared.
4. A bind error puts nothing back in the map. The credential file is unchanged when `bind` returns `Err` before it writes. A remote HTTP 500 is `500` and `unknown_error`.
5. `GET /api/binding` reads the file. `token_stored` is `true` when `project_token` is set, otherwise `false`.
6. `GET /api/keys` is `200` and `data` `[]`. `POST /api/keys` and `POST /api/keys/print` are `501` and `keys_later`.

`api.ts` calls select, then the exhibition screen shows `token_stored` and `keys.empty`. A Vitest mocks those two responses.

### Test scenarios

| Scenario | Assert |
|---|---|
| `select_stores_token` | Stand-in `profile/synchdev` returns a token. After select, the credential file has that `expo_id`, the posted name, and that `project_token`. `data.token_stored` is `true`. The token string is not in the JSON. |
| `select_drops_session` | The same cookie on a following `GET /api/exhibitions` is `401`. `GET /api/binding` still reports `token_stored` `true` and that expo id. |
| `select_bind_failure_keeps_file` | Stand-in synchdev returns HTTP 500. The credential file still has no `project_token`. The cookie is gone. |
| `keys_empty` | `GET /api/keys` is `200` and `data` `[]`. |
| `create_key_not_implemented` | `POST /api/keys` is `501` and code `keys_later`. |
| `screen_shows_stored_and_empty_keys` | Mocked select and keys responses render the stored sentence and `keys.empty`. |

### Done when

`cargo test --test admin_pages select_exhibition` and `npm test` pass, and steps 8–10 stay green.

## Step 12. Mounted server

[Back to summary](#summary)

The admin API and the built files are on the router `serve` already runs, beside `/health`, `/registrations`, `/print`, and `/ws`.

### Work

1. Merge the admin routes into `http::router`. `/api` is matched before the static fallback.
2. `GET /` and `GET /exhibitions` return `index.html` from the build directory, with the same validators as any other static file.
3. `Config` carries the dist path. Tests pass a temp directory that contains `index.html` and `assets/app.js`. The real `npm run build` output is what the process serves.
4. The existing inflight gate wraps these routes the same way it wraps `/health`.
5. `App.tsx` routes `/` to `Login` and `/exhibitions` to `Exhibitions`, using `api.ts`.

### Test scenarios

Start `serve` the way `tests/registration_server.rs` does, with a temp dist and a stand-in remote.

| Scenario | Assert |
|---|---|
| `serve_index` | `GET /` and `GET /exhibitions` are `200` and the temp `index.html` bytes. |
| `serve_asset_304` | `GET /assets/app.js` then the same GET with `If-None-Match` is `304`. |
| `serve_login` | `POST /api/login` against the stand-in is `200` and sets the cookie. |
| `health_still_ok` | `GET /health` is still `{"status":"ok"}`. |
| `api_is_not_index` | `GET /api/missing` is `404`, code `not_found`, and the body is not `index.html`. |

### Done when

`cargo test --test admin_pages serve_admin`, `cargo test --test registration_server`, and `npm test` pass, and steps 1–11 stay green. `npm run build` writes `registration-admin/dist`.

## Following plan — exhibition sqlite and keys

Not steps 1–12. Start this after step 12 is done, before [porting.md](porting.md) step 4 puts the token on the sync round.

`setCurrentExpoUID` in the Scala client opens a different on-disk database for the chosen exhibition. The port does the same at `bind`: close the previous file, open the file for the new `expo_id`. Keys are rows in that file. The credential file still holds only `device_id`, `base_url`, `expo_id`, `expo_name`, and `project_token`.

Step 11 left the key routes empty. This plan replaces those handlers. Responses use the same `{ "ok", "data" }` / `{ "ok", "error": { "code" } }` envelope. The exhibition screen fills `keys` from `GET /api/keys` instead of the empty sentence:

| route | `data` |
|---|---|
| `GET /api/keys` | rows `{ "id", "key", "comment", "is_admin" }` |
| `POST /api/keys` | insert an operator key, return `{ "id", "key" }` |
| `PATCH /api/keys/:id` | update key text and comment |
| `DELETE /api/keys/:id` | delete by id |
| `POST /api/keys/print` | one print job for the current keys, on the server-thread print queue |

Switching exhibition switches the file. Keys from the previous exhibition stay in the previous file. New strings go into `ru.json` under `keys.*`.
