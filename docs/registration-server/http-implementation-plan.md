# Local HTTP API — step 9

Build sequence for [porting.md](porting.md) step 9. The event file is [db-design.md](db-design.md). Print jobs are [printing.md](printing.md). Login, the event list, and the operator cookie are [admin-implementation-plan.md](admin-implementation-plan.md).

This is the last server step. It replaces `RegApp.scala`. Every old route is listed below. A route that served a mustache page is not served. A route that returned JSON, a file, or a print is an `/api` route on the server thread. The screens that call these routes are [porting.md](porting.md) step 10. That app is tested against fixtures and does not wait on this step.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 9 is done when steps 1 through 6 are done. The scan API and the leadgen API are deprecated. They are not ported.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Liveness](#step-1-liveness) | The device is up, and which event is open | done |
| [2. Forms](#step-2-forms) | Config, enums, backgrounds, categories, tickets | done |
| [3. Visitors](#step-3-visitors) | Search, one row, save, photo, counts | done |
| [4. Print](#step-4-print) | Chosen printer, zone, certificate, and a setup preview | done |
| [5. Desk](#step-5-desk) | Keys, printers, the registration flags | done |
| [6. Sync status](#step-6-sync-status) | How many rows are waiting, and a manual wake | done |

Shared rules:

- Routes live in `src/registration_server/http.rs`. Success is `{ "ok": true, "data": ... }`. Failure is `{ "ok": false, "error": { "code": "..." } }`. That is the envelope step 3 already uses.
- Floor routes do not require the operator cookie. They read and write the open event. Operator routes require that cookie. An admin-only route also requires the desk key from step 5 to be an admin key, except the routes step 3 already guards with the operator cookie alone (`/api/keys` create stays the operator cookie, as the admin plan specifies).
- A handler does its work and returns. It does not parse a remote payload and it does not render a badge inline. A print enqueues on the step 8 worker and returns the job id. Search uses the memory database from the db plan.
- The skeleton `GET /registrations/{id}` and `POST /print` are removed when the routes below replace them. `GET /health` stays. `GET /ws` stays the idle lookup socket from [design.md](design.md). `GET /`, `GET /events`, and the `/api/login` family stay as step 3 built them.
- Tests use a temp event directory and `CurrentEvent`. They do not call kuprin.su and they do not start cupsd. A print test uses the IPP stand-in from the printing plan.
- Step 3 stays green: `cargo test --test admin_pages`. Step 8 of printing stays green when that plan has landed: `cargo test -p ticket-render` and `cargo test --lib registration_server::cups`.

## Every old route

`RegApp.scala` is the source. The new method and path are what this step serves. "Page" means the process returns 404 for that path. The UI is a React route, not this process.

### Floor

| Old route | New route | What it does |
|---|---|---|
| `GET /is_reg_alive` | `GET /health` | Empty 200. The body stays `{ "status": "ok" }`. |
| `GET /expoid` | `GET /api/event` | The open `event_id`, or an empty id when none is open. |
| `POST /datapost` | `POST /api/registrations` | Save one registration and enqueue its badge. |
| `POST /userdata` | `GET /api/registrations` | Search, or one stored row. |
| `POST /get_mail_list` | `GET /api/registrations?code=` | The rows for one scanned barcode. |
| `GET /userlist?json=true` | `GET /api/registrations` | The list page. The HTML branch of `/userlist` is a page. |
| `GET /user` | `GET /api/registrations/{id}` | The stored JSON for one uid. |
| `POST /photopost` | `POST /api/registrations/{id}/photo` | Store the photo bytes for that registration. |
| `GET /newPhoto` | `GET /api/registrations/{id}/photo` | Those bytes, or 404. |
| `GET /emailCount` | `GET /api/registrations/counts?email=` | How many rows share that email. |
| `GET /phoneCount` | `GET /api/registrations/counts?phone=` | How many rows share that phone. |
| `POST /printfromlist` | `POST /api/print` | Print the named uids. `GET /printfromlist` is the same call and is not a second route. |
| `POST /askforcert` | `POST /api/certificates` | Reserve a certificate code and enqueue that layout. |
| `POST /setgotsome` | `PATCH /api/registrations/{id}` | Set `gotsome`. |
| `GET /setpayed` | `PATCH /api/registrations/{id}` | Set the ticket status paid. |
| `GET /regapp/vars` | `GET /api/forms/vars` | The form variable map sync stored. |
| `GET /regapp/user_struct` | `GET /api/forms/{id}/struct` | The field structure for that form. |
| `GET /regapp/user_model` | `GET /api/forms/{id}/model` | The model for that form. |
| `GET /regapp/quest_conf` | `GET /api/forms/{id}/conf` | The question config for that form. |
| `GET /regapp/categories` | `GET /api/forms/categories` | Category id and name. |
| `GET /regapp/global_settings` | `GET /api/forms/{id}/settings` | Global settings for that form. An absent id is the event-level settings. |
| `GET /regapp/bgs/:bgname` | `GET /api/forms/backgrounds/{name}` | The background file sync stored. |
| `GET /qrlink` | `GET /api/qr?text=` | PNG bytes from `ticket-render` `render_qr`. |
| `GET /regapp/lottery_wins` | `GET /api/lottery` | Lottery rows for the form. |
| `GET /dbenum/:loc/:enumname` | `GET /api/enums/{locale}/{name}` | The enum file sync stored. |

### Desk

These require the operator cookie. The old `protect(AdminKey)` routes also require an admin desk key.

| Old route | New route | What it does |
|---|---|---|
| `GET /keys` | `GET /api/keys` | The key rows. Replaces the empty list from step 3. |
| `POST /createKey` | `POST /api/keys` | Insert an operator key. Replaces `501` `keys_later`. |
| `POST /editKey` | `PATCH /api/keys/{id}` | Update the text and the comment. |
| `POST /deleteKey` | `DELETE /api/keys/{id}` | Delete that row. |
| `POST /printKeys` | `POST /api/keys/print` | Enqueue one badge per key. |
| `POST /keyAuth` | `POST /api/desk/auth` | Check a desk key. Set the desk cookie. `GET /keyAuth` is a page. |
| `GET /get_printers` | `GET /api/printers` | Connected printers, choices, values, routing. |
| `GET /printers/all` | `GET /api/printers` | Names are a field on that list. `GET /printers/all2` is the same list with make and model. |
| `GET /update_printers` | `POST /api/printers/search` | Run the step 8 search and return the new list. |
| `POST /set_printer_params` | `PATCH /api/printers/{name}` | Save `values` to `base/printer_conf/{name}.yml`. `GET /set_printer_params` is the same body and is not a second route. |
| `POST /set_printer_routings` | `PATCH /api/printers/{name}/routing` | Save `used`, `zone`, `category`, `is_internet`, `print_cert`, `sector_name`, `second_name`, `auto_print`, `no_postscript`, and `rule` for the open event. |
| `GET /test_printer` | `POST /api/printers/{name}/test` | Enqueue the test badge from the printing plan. |
| `GET /printers/jobs/cancel-all` | `POST /api/printers/cancel` | `Cancel-Job` for jobs this process submitted, and drop the waiting print channel. |
| `POST /show_ticket_status` | `PATCH /api/settings` | The registration flags, one object. |
| `POST /show_gotsome` | `PATCH /api/settings` | Same object. |
| `POST /print_from_ipad_form` | `PATCH /api/settings` | Same object. |
| `POST /capitalize_names` | `PATCH /api/settings` | Same object. |
| `POST /doprinttranslit` | `PATCH /api/settings` | Same object. |
| `POST /dodoubleprint` | `PATCH /api/settings` | Same object. |
| `POST /isloadingbarcodes` | `PATCH /api/settings` | Same object. `GET /isloadingbarcodes` reads it. |
| `GET /barcodes/conf` | `GET /api/barcodes` | Remaining pool counts per category. |
| `POST /barcodes/conf` | `PATCH /api/barcodes` | The min and add counts the sync plan reads. |
| `GET /countnotsynched` | `GET /api/sync` | How many local rows are still waiting to go up. |
| `GET /synchronizer/task/add` | `POST /api/sync` | Wake the sync thread once. |
| `GET /db_view/get_give_packets` | `GET /api/packets` | Packet rows for the open event. |
| `POST /db_view/give_packets` | `POST /api/packets` | Mark a packet given. |
| `GET /printed_num` | `GET /api/prints/summary` | Print counts. `printed_num_old` and `printed_num_bad` are the same summary with the filter the query names. |

### Pages, not served

| Old route | Why it is not served |
|---|---|
| `GET /`, `GET /pay`, `GET /regscreens`, `GET /confirmprocessing` | The registration shell. `GET /` is already the React `index.html` from step 3. |
| `GET /admin`, `POST /admin` | Login is `POST /api/login`. |
| `GET /controlExpo`, `GET /exposlist`, `POST /selectexpo`, `GET /logout` | The event screen is step 3: `GET /api/events`, `POST /api/events/select`. |
| `GET /keyAuth`, `GET /operator`, `GET /form`, `GET /db_view`, `GET /settings`, `GET /settings_registration`, `GET /test_form`, `GET /printers`, `GET /start/:lng` | Mustache. The new screens call the desk routes above. |
| `GET /userlist` without `json=true` | The HTML list. The JSON list is `GET /api/registrations`. |
| `GET /reload` | The handler writes an empty 200 and reloads nothing. |

### Not this port

| Old route | Why it stays out |
|---|---|
| `GET /updatecheck`, `GET /version`, `GET /update`, `GET /need_update` | [porting.md](porting.md) step 11. These four paths stay unserved. The desk uses `GET /api/update` and `POST /api/update`. |
| `POST /scannedbc`, `GET /control_zones`, `GET /possible_tickets`, `POST /tracked`, `GET /was_scanned` | The scan API is deprecated. |
| `POST /leadgen/download`, `POST /leadgen/upload`, `POST /locallogin/getzones` | The leadgen API is deprecated. |
| `GET /cameraCap`, `POST /uploadphototest` | The local camera test page. A photo the floor already took is `POST /api/registrations/{id}/photo`. |
| `GET /uploadbarcodes`, `POST /uploadbarcodes`, `GET /uploadvisitcodes`, `POST /uploadvisitcodes` | A hand upload of barcode files. Sync already downloads `boxapi/barcodes`. |
| `POST /uploadcsv`, `POST /set_separator`, `POST /csv_setup`, `POST /remove_last_csv` | The CSV importer behind a mustache page. |
| `GET /users/find_dublicate`, `GET /users/merge_dublicate` | A one-off merge tool. |
| `GET /db/memory/backup` | Copies the working database to a file. The event file is already the durable copy. |
| `GET /setdate`, `GET /getdate` | Sets the device clock. |

## How the served routes work

### Event and health

`GET /health` is the skeleton. `GET /api/event` reads `CurrentEvent`. The data is `{ "id" }` and `id` is null when the holder is empty. It does not read the credential file's token.

### Forms

Sync writes the form config, the backgrounds, the enum files, and the categories into the event directory. These routes read those files and return the JSON. A missing file is 404 `not_found`. `GET /api/forms/backgrounds/{name}` returns the file bytes with the image content type. The query carries `locale`, `form_id`, and `resolution` (`pad` or `phone`), which is what `getBgFile` selected. `GET /api/enums/{locale}/{name}` returns the stored JSON, gunzipped when the file on disk is gzip. `GET /api/qr` calls `render_qr` and returns the PNG. It does not write `./qr/link.png`.

### Visitors

`GET /api/registrations` is the db plan's search and list page. Query fields are `q`, `name`, `surname`, `email`, `company`, `category`, `code`, `limit`, and `offset`. `code` is the barcode lookup `/get_mail_list` did. One exact uid is `GET /api/registrations/{id}` and returns the stored JSON row. The list data includes `show_gotsome` and `show_ticket_status` from the settings object.

`POST /api/registrations` is `datapost`. The body is the visitor JSON the floor already posts as `data`. The handler writes the row through the db plan's write path, takes a ticket barcode when the row needs one and the pool has one, and enqueues a print when the settings say the floor prints on save and the body does not say `noprint`. An empty pool leaves the barcode empty and still writes the row. The data on success is `{ "zone_name", "ticket_status" }`. `zone_name` is the sector name of the queue the print pick chose. `ticket_status` is present when the settings show it.

An email plus a phone that hit different rows is 409 `invalid_phone_email_combo`, which is the combo check inside `/userdata`. When the form settings say one row per email or per phone and the search hits one row, the response is that row instead of a list. The handler does that before insert.

`POST /api/registrations/{id}/photo` stores the multipart bytes under the event's `photos/` directory and marks the row dirty so sync will send `boxapi/uploadphoto`. `GET` returns the bytes.

`GET /api/registrations/counts` returns `{ "count" }` for `email` or `phone`.

`PATCH /api/registrations/{id}` accepts `gotsome` and `ticket_status`. Those are the `/setgotsome` and `/setpayed` writes. The row is marked dirty for `uploadreg`.

### Print

`POST /api/print` replaces the skeleton body. The body is `{ "ids", "printer", "zone", "certificate", "category", "noprint", "check" }`. `ids` are the uids. The handler loads each row and passes `printer`, `zone`, and `certificate` to the step 8 pick, then enqueues.

`printer` is the chosen queue name. When it is set, the pick returns that queue. `zone` is the routing zone. When `printer` is empty and `zone` is set, the pick keeps queues whose routing `zone` matches. `certificate` is true when the job is a certificate. The pick then keeps queues whose routing `print_cert` is set and draws the certificate layout. A false or absent `certificate` uses the category printer. `category` on the body overrides the category on the stored row. The internet flag is the flag on that row. The body does not send a zone rule. A missing `rule` on the queue is `round_robin`.

`noprint` writes the print row and does not enqueue. `check` is the `doCheck` flag: a uid that fails the check is listed in the response and is not printed. The data is one object per queue that received a job: `{ "printer", "zone", "certificate", "zone_name" }`. `printer` is the queue the pick chose. `zone` is that queue's routing zone. `certificate` is the flag the job carried. `zone_name` is the sector name, the string the old client put in `zoneName.ru`. A pick that matches nothing is 409 `no_printer`.

`POST /api/certificates` takes `ids`, `printer`, and `zone`. It reserves a certificate code the way `askForCerts` did and enqueues the same pick with `certificate` true. The data is `{ "name", "code", "printer", "zone" }` per id. No code left uses the code string the old handler returned when the pool was empty, under `code`, and does not enqueue.

`POST /api/print/preview` takes `{ "id", "printer", "zone", "certificate" }`. It picks with those fields, returns the PNG for that queue, and does not submit a job. A visitor preview sends the id. Printers setup sends `printer` and no id. That call returns the test-page PNG for the named queue and does not enqueue. `POST /api/printers/{name}/test` still enqueues the test badge.

### Desk

`POST /api/desk/auth` reads `{ "key" }`. A known key sets an HttpOnly desk cookie. An unknown key is 401 `invalid_key`. Admin-only routes check that cookie's key against `is_admin`.

`GET /api/keys` returns `{ "id", "key", "comment", "is_admin" }`. `POST /api/keys` inserts an operator key and returns `{ "id", "key" }`. `PATCH` and `DELETE` take the id. `POST /api/keys/print` enqueues one test-shaped badge per key, with the key text as the name and the comment as the company. The two default rows from step 4 are the rows an empty table already has.

`GET /api/printers` is the in-memory printer list from step 8, including a queue with `connected` false so the screen can show it. Each queue includes its routing record. `POST /api/printers/search` runs search and then returns that list. `PATCH /api/printers/{name}` writes `values`. `PATCH /api/printers/{name}/routing` writes `used`, `zone`, `category`, `is_internet`, `print_cert`, `sector_name`, `second_name`, `auto_print`, `no_postscript`, and `rule`. A field the body omits stays as stored. `rule` is `round_robin` or `fewest_jobs`. A missing rule is `round_robin`. The pick reads `used`, `zone`, `category`, `is_internet`, `print_cert`, and `rule` from that record. `POST /api/printers/{name}/test` enqueues the test page. `POST /api/printers/cancel` cancels submitted IPP jobs and drops jobs still waiting on the print channel.

`GET /api/settings` returns one object: `show_ticket_status`, `show_gotsome`, `print_on_save`, `capitalize_names`, `transliterate`, `double_print`, `load_barcodes`. `PATCH` writes the fields it is given and leaves the others. The file is `base/{event_id}/settings.json`. The print worker reads `transliterate` and `double_print` when it decides on a second page.

`GET /api/barcodes` is the per-category remaining counts and the min and add settings. `PATCH` writes min and add. Sync reads those numbers when it asks for the next barcode page.

`GET /api/packets` lists give-packet rows. `POST /api/packets` marks one given.

`GET /api/prints/summary` returns counts of printed rows. The query `which` is `current`, `old`, or `bad`, matching the three old pages.

### Sync status

`GET /api/sync` returns `{ "waiting" }`, the count of rows with `in_synch` unset that still have to go up. `POST /api/sync` asks the sync thread to wake once. It does not itself open a socket to kuprin. A wake that finds the queue busy does not fill it again, which is the sync plan's rule.

## Step 1. Liveness

[Back to summary](#summary)

The floor can see that the process is up and which event is open.

### Work

1. `GET /health` stays `{ "status": "ok" }`.
2. `GET /api/event` returns the open id.

### Test scenarios

| Scenario | Assert |
|---|---|
| `health_stays_ok` | `GET /health` is 200 and `{ "status": "ok" }`. |
| `event_id_follows_the_holder` | With an open event, `GET /api/event` data is that id. After `clear`, `id` is null. |

### Done when

`cargo test --lib registration_server::http::tests::liveness` passes, and `cargo test --test admin_pages` stays green.

## Step 2. Forms

[Back to summary](#summary)

The floor reads the form files sync stored.

### Work

1. Serve vars, struct, model, conf, categories, settings, backgrounds, enums, and the QR PNG as in the table.
2. A missing file is 404 `not_found`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `form_json_is_the_stored_file` | A temp event with `forms/1/conf.json` returns that JSON from `GET /api/forms/1/conf`. A missing form is 404 `not_found`. |
| `background_is_the_file_bytes` | `GET /api/forms/backgrounds/bg` with `locale` and `resolution` returns the PNG bytes stored for that pair. |
| `qr_is_png` | `GET /api/qr?text=abc` returns a PNG. Decoding it yields `abc`. |

### Done when

`cargo test --lib registration_server::http::tests::forms` passes, and step 1 stays green.

## Step 3. Visitors

[Back to summary](#summary)

Search, one row, save, photo, and the email and phone counts.

### Work

1. `GET /api/registrations` and `GET /api/registrations/{id}` use the db plan.
2. `POST /api/registrations` writes one row. An empty ticket pool leaves the barcode empty and still writes the row. The email and phone combo check returns 409 `invalid_phone_email_combo`.
3. Photo POST and GET use `photos/` under the event directory.
4. Counts return a number.

### Test scenarios

| Scenario | Assert |
|---|---|
| `search_by_code` | A stored barcode is returned by `GET /api/registrations?code=`. An unknown code is an empty list. |
| `save_writes_the_row` | `POST /api/registrations` inserts a row the following GET returns. The print flag off does not enqueue. |
| `empty_barcode_still_saves` | An empty ticket pool writes the row and leaves the barcode unset. |
| `photo_roundtrip` | POST bytes are the GET bytes. A missing photo is 404. |

### Done when

`cargo test --lib registration_server::http::tests::visitors` passes, and step 2 stays green.

## Step 4. Print

[Back to summary](#summary)

Printing a list uses the step 8 worker. The job carries the chosen printer, the zone, and whether it is a certificate. Printers setup previews one named queue.

### Work

1. `POST /api/print` loads the ids and passes `printer`, `zone`, and `certificate` to the pick, then enqueues. The response names the queue that received the job, its zone, and the certificate flag.
2. `POST /api/certificates` reserves a code and enqueues with `certificate` true and the same `printer` and `zone`.
3. `POST /api/print/preview` returns the PNG for that pick and does not submit. A body with `printer` and no id is the printers-setup preview.

### Test scenarios

| Scenario | Assert |
|---|---|
| `print_uses_the_chosen_printer` | `printer` "HP" produces one `Print-Job` on that queue. The data `printer` is "HP". `noprint` produces no `Print-Job` and still returns 200. |
| `zone_and_certificate_choose_the_queue` | An empty `printer` and `zone` "hall" sends the job to a queue in that zone. `certificate` true sends it to the certificate printer in that zone. The data reports that printer, that zone, and `certificate` true. |
| `no_printer_is_conflict` | No connected queue is 409 `no_printer`. |
| `preview_is_png` | A visitor preview and a printers-setup preview (`printer` set, no id) each return PNG bytes. The stand-in sees no `Print-Job`. |
| `certificate_reserves_a_code` | One ticket barcode is returned as `code`, and the job goes to the certificate printer. An empty pool returns an empty `code` and does not enqueue. |

### Done when

`cargo test --lib registration_server::http::tests::print` passes, and step 3 stays green.

## Step 5. Desk

[Back to summary](#summary)

Keys, printers, and the registration flags. The operator cookie is required.

### Work

1. Replace the step 3 key stubs with the table reads and writes.
2. Desk auth sets the desk cookie. Admin-only printer and settings routes check it.
3. Printer routes call the step 8 records. Settings are one YAML file, with `class` `native` or `ipp`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `keys_roundtrip` | `POST /api/keys` then `GET /api/keys` includes the new operator key. `DELETE` removes it. The two default rows remain. |
| `desk_key_required` | `PATCH /api/settings` without a desk cookie is 401. With an admin key it stores `double_print` true, and a following GET returns it. |
| `printer_patch_writes_the_file` | `PATCH /api/printers/{name}` writes `base/printer_conf/{name}.yml`, including `class`. Search is not sent to cupsd by the PATCH. |

### Done when

`cargo test --lib registration_server::http::tests::desk` passes, `cargo test --test admin_pages` stays green, and step 4 stays green.

## Step 6. Sync status

[Back to summary](#summary)

The desk can see the outbox and ask for one wake.

### Work

1. `GET /api/sync` counts rows still waiting to go up.
2. `POST /api/sync` wakes the sync thread once.

### Test scenarios

| Scenario | Assert |
|---|---|
| `waiting_count` | One unsynced registration makes `waiting` 1. After it is marked synced, `waiting` is 0. |
| `wake_once` | `POST /api/sync` causes one wake. A second POST while that wake is in the busy queue does not open a second probe. |

### Done when

`cargo test --lib registration_server::http::tests::sync_status` passes, `cargo test --lib registration_server::sync::tests::queue` stays green, and steps 1 through 5 stay green.
