# Registration UI — operator screens

Build sequence for the screens in [registration-ui-design.md](registration-ui-design.md). The form engine is already [form-implementation-plan.md](form-implementation-plan.md). This plan adds the desk key, the menu, and the screens that menu opens. It does not rebuild the form.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing, including the form plan's `npm test`. Set **Status** to `not started`, `in progress`, or `done`.

The operator screens are done when steps 1 through 12 are done. Tests fetch the `/api` paths below through a stub. They do not start `rust-reg` and do not call cupsd.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Shell](#step-1-shell) | Routes, the desk-cookie gate, Back and Forward | done |
| [2. Lazy routes](#step-2-lazy-routes) | A screen module loads when the current screen links to it | done |
| [3. Key](#step-3-key) | The key field and `POST /api/desk/auth` | done |
| [4. Menu](#step-4-menu) | Four tiles, the sync count, the QR | done |
| [5. Form path](#step-5-form-path) | `/form` renders the existing desk component | done |
| [6. Visitors](#step-6-visitors) | Search, category, the table, the printer | done |
| [7. Print confirm](#step-7-print-confirm) | Packets, enqueue, then `/print` | not started |
| [8. Print progress](#step-8-print-progress) | The running job, pause, stop | not started |
| [9. Import](#step-9-import) | Column mapping and the CSV file on one screen | not started |
| [10. Settings](#step-10-settings) | The two settings links | not started |
| [11. Printers](#step-11-printers) | Queues, test, save | not started |
| [12. Registration settings](#step-12-registration-settings) | The six switches, barcode pools, sync | not started |

Shared rules:

- The app stays `registration-form/`. React Router runs in framework mode. `app/routes.ts` lists each screen as its own module file. `app/root.tsx` does not import those modules.
- `FormClient` stays the only module that knows URLs. New calls are methods on it. Every fetch sends credentials, so the HttpOnly desk cookie goes with the request. The app does not read `document.cookie`.
- An operator URL is everything in the route table except `/register` and `/key`. The operator layout loader calls `GET /api/sync`. Status 401 redirects to `/key`. Status 200 renders the screen. A later desk call that returns 401 redirects to `/key` the same way. `/register` has no such loader.
- After a 200 from `POST /api/desk/auth`, every operator path renders. The menu shows all four tiles for every accepted key.
- Chrome strings live in `app/locales/{ru,en}.json`. A missing chrome key fails the test run. Tests render through `react-i18next` and assert the Russian labels from the design.
- Each operator screen keeps `data-screen` on its root: `key`, `menu`, `form`, `visitors`, `print`, `import`, `settings`, `printers`, `registration`.
- A test that renders uses Testing Library and jsdom. The command is `npm test` in `registration-form/`. The filter is the name in **Done when**.
- Pages the design leaves out stay out: login, the event list, key editing, the device clock, system restart, duplicate merge, and a hand upload of barcode files. The binary update is [porting.md](porting.md) step 11.

## Step 1. Shell

[Back to summary](#summary)

The route table exists, and a missing desk cookie opens the key screen. Screen bodies are titles only. Later steps fill them in.

### Work

1. `routes.ts` adds the modules from the design. `/form` points at `routes/desk.tsx`. `/desk` redirects to `/form`.
2. `/register` stays the kiosk route from the form plan. It is outside the operator layout.
3. The operator layout calls `GET /api/sync` before the screen renders. A 401 response redirects to `/key`. A 200 response renders the matched screen. `/key` does not call that loader.
4. Any desk call that returns 401 after the screen is open redirects to `/key`.
5. The header on every operator screen has Back and Forward. Back is a link to the path in the design's route table. Forward is disabled. `/key` and the menu have Back disabled, because those screens are the start of the desk.
6. Each new module renders its `data-screen` value and a heading, so this step can tell the routes apart. `routes/desk.tsx` keeps rendering the form it already renders, and gains `data-screen="form"`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `shell_missing_cookie_opens_the_key` | `/`, `/form`, `/visitors`, `/print`, `/import`, `/settings`, `/printers`, and `/settings/registration` each redirect to `/key` when `GET /api/sync` is 401. The address bar after the redirect is `/key`. |
| `shell_cookie_opens_every_operator_path` | The same paths render their own `data-screen` when `GET /api/sync` is 200. `/desk` ends on `/form`. |
| `shell_register_ignores_the_desk_cookie` | `/register` with that 401 still shows "Начать". It does not redirect to `/key`. |
| `shell_later_401_returns_to_the_key` | A screen that loaded with a 200 sync, then receives 401 from its next desk call, is replaced by `/key`. |
| `shell_back_follows_the_table` | On `/visitors`, Back points at `/`. On `/print`, Back points at `/visitors`. On `/printers` and `/settings/registration`, Back points at `/settings`. Forward is disabled on each of those screens. |

### Done when

`npm test -- shell_` passes, and `npm test` from the form plan stays green.

## Step 2. Lazy routes

[Back to summary](#summary)

A screen's module is fetched when a link to it is on the screen the operator is viewing. Opening a URL still loads that one module. The other screens stay unloaded.

### Work

1. Every `Link` sets `prefetch="viewport"`.
2. The route modules stay separate files. The menu module does not import the form, visitors, import, or settings modules. Those arrive through the router.
3. The test router records each dynamic import. Its `IntersectionObserver` reports every observed link as in view, which is what `prefetch="viewport"` waits for in jsdom.

### Test scenarios

| Scenario | Assert |
|---|---|
| `lazy_key_loads_no_other_screen` | Rendering `/key` records `routes/key.tsx` only. |
| `lazy_menu_loads_the_tiles` | Rendering `/` with a cookie records `routes/menu.tsx`, then `routes/desk.tsx`, `routes/visitors.tsx`, `routes/import.tsx`, and `routes/settings.tsx`. It does not record `routes/print.tsx`, `routes/printers.tsx`, or `routes/registration-settings.tsx`. |
| `lazy_form_loads_the_menu` | Rendering `/form` records `routes/desk.tsx` and, from the Back link, `routes/menu.tsx`. It does not record `routes/visitors.tsx`. |
| `lazy_settings_loads_its_two_links` | Rendering `/settings` records `routes/printers.tsx` and `routes/registration-settings.tsx`. Rendering `/printers` records `routes/settings.tsx` from Back and does not record `routes/registration-settings.tsx`. |
| `lazy_typed_url_loads_that_screen` | Opening `/print` directly records `routes/print.tsx` and `routes/visitors.tsx` from Back. It does not record `routes/import.tsx`. |

### Done when

`npm test -- lazy_` passes, and step 1 stays green.

## Step 3. Key

[Back to summary](#summary)

`3.html` and `keyAuth.mustache`. One field and one button.

### Work

1. `FormClient.auth(key)` posts `{ "key" }` to `POST /api/desk/auth`.
2. The screen shows a text field with placeholder "Ключ" and a button "Войти".
3. An empty field does not post.
4. Status 401 `invalid_key` stays on `/key` and shows the error from the catalog.
5. Status 200 navigates to `/`. There is no link on this screen, so the menu module loads at that navigation.
6. The screen has no list of keys, no create control, and no print-keys control.

### Test scenarios

| Scenario | Assert |
|---|---|
| `key_shows_the_field` | The placeholder is "Ключ" and the button is "Войти". |
| `key_empty_does_not_post` | Submitting an empty field leaves the URL on `/key` and records no `POST /api/desk/auth`. |
| `key_unknown_stays` | `POST /api/desk/auth` 401 `invalid_key` leaves the URL on `/key` and shows the error text. The field keeps what was typed. |
| `key_accepted_opens_the_menu` | A 200 response navigates to `/` and the menu `data-screen` is shown. The recorded imports gain `routes/menu.tsx` at that navigation. |
| `key_has_no_key_admin` | The screen has no control whose name is a key row, "Сохранить" for a key, or printing keys. |

### Done when

`npm test -- key_` passes, and steps 1 and 2 stay green.

## Step 4. Menu

[Back to summary](#summary)

`4.html` and `operator.mustache`. Four tiles, the waiting count, and the QR.

### Work

1. Four links, in this order: "Оператор" to `/form`, "Печать" to `/visitors`, "Загрузить базу" to `/import`, "Настройки" to `/settings`. All four render after any accepted key.
2. `GET /api/sync` is the `{ "waiting" }` count already used as the cookie probe. The screen shows that number.
3. The QR image is `GET /api/qr?text=` plus the origin and `/register`.
4. The screen has no "Обновить" control, no "Включить модерацию" checkbox, and no list of local addresses.

### Test scenarios

| Scenario | Assert |
|---|---|
| `menu_shows_four_tiles` | The four labels are links to `/form`, `/visitors`, `/import`, and `/settings`. |
| `menu_shows_the_waiting_count` | A sync body `{ "waiting": 3 }` shows 3. |
| `menu_shows_the_register_qr` | The image URL contains `/api/qr` and the encoded `/register` path. |
| `menu_omits_update_and_moderation` | There is no "Обновить" control and no "Включить модерацию" checkbox. |

### Done when

`npm test -- menu_` passes, and steps 1 through 3 stay green.

## Step 5. Form path

[Back to summary](#summary)

The form screen is the desk component from the form plan. This step gives it the path and the Back link from the design.

### Work

1. `/form` renders `Desk` from `routes/desk.tsx`. The header, the step, Save, the printer, and the packets stay as the form design specifies.
2. Back on this screen points at `/`.
3. `/desk` redirects to `/form`.
4. `5.1-if-zero` and `5.2` stay states of this component. This step adds no route for them.

### Test scenarios

| Scenario | Assert |
|---|---|
| `formpath_renders_the_desk` | `/form` with a cookie shows the operator Save control and `data-screen="form"`. |
| `formpath_back_opens_the_menu` | Back on `/form` points at `/`. Following it shows the four menu tiles. |
| `formpath_desk_redirects` | `/desk` ends on `/form` and shows that same Save control. |
| `formpath_register_is_unchanged` | `/register` still shows "Начать" and does not show the desk Save control. |

### Done when

`npm test -- formpath_` passes, `npm test -- desk_` passes, and steps 1 through 4 stay green.

## Step 6. Visitors

[Back to summary](#summary)

`7.html` and the list half of `db_view.mustache`. Search and the table. The print dialog is the next step.

### Work

1. `FormClient.registrations(query)` calls `GET /api/registrations` with `q`, `category`, `limit`, and `offset`. `FormClient.printers()` is the list already on the client.
2. One search field. A category multi-select whose first option is "Все категории". The other options are the shared desk catalog. "Все категории" is selected alone and sends no `category`. Each other category adds its id, and choosing it again removes that id. The choice does not change the category stored for the form.
3. Columns are a row checkbox, Имя, Фамилия, Компания, Категория, and `(print_count)` at the right. The cells are `name`, `surname`, `c_name`, `category`, and `print_count`. The header checkbox selects or clears the current page. Those checks clear on a page change, a search change, or a category change. "Выбрать все" selects every match of the current search or category, stays selected across pages, and is cleared when the search or category changes. It is disabled when both are empty. An empty `c_name` shows `email` in that cell, grey: the first two letters, then exactly `***`, then `@` and the domain. A shorter or empty local part still gets `***`. Even and odd rows use two close backgrounds. `print_count` above zero uses a light amber pair.
4. "<" and ">" on the bottom bar ask for the previous and next `offset`. Between them are buttons for the first page, the last page, and pages `current - 2` through `current + 2`. A gap in that run is "...". The current page button is selected. "<" is disabled at offset 0. ">" is disabled when `offset + limit` reaches `total`. The bottom-bar buttons are compact so the pager and the actions stay on one row.
5. The shared printer dropdown sits in the desk navigation, the same custom list as the form. The chosen name stays selected for the confirm step.
6. The bottom bar holds the pager, then "Отметить", "Заказать сертификат", and "Печать". "Отметить" and "Заказать сертификат" are toggles. Their pressed state stays for the confirm step.
7. There is one search field. When it has text, a round clear button at its right edge empties it. The separate email, name, surname, and company inputs from the static page are not on this screen.
8. The desk navigation does not reserve a blank gap for a Посетитель tab. The printer sits immediately after Посетители on this list only, and the network status fills the rest of the bar. Opening a visitor removes that header printer and adds the Посетитель tab. The first click on a row highlights it. The second click on that row opens the tab at `/visitor?userId={uid}`, which is the form filled from `GET /api/registrations/{uid}`. Reloading that address keeps the tab open and loads the same visitor. Leaving `/visitor` for another screen closes the tab. The close control returns to `/visitors` and clears the tab. If the form changed since it opened or was last saved, the close and the switch ask whether to continue and drop those changes. Сохранить on that form writes the visitor and closes the tab. Печать writes first only when that same change check says the form changed, posts `POST /api/print` for that uid, and closes the tab after the print is accepted. The printer label in that bar is an icon, and the network status grows into the space left beside it.

### Test scenarios

| Scenario | Assert |
|---|---|
| `visitors_lists_rows` | Rows from `GET /api/registrations` show name, surname, company, category, and `(print_count)`. A row with no company shows a grey masked email in the company cell. Even and odd rows differ. A printed row uses the amber tint. |
| `visitors_search_and_category` | Typing in the search field and choosing a category sends `q` and `category`. A second category is added. Choosing a selected category removes it. "Все категории" clears the others and omits `category`. The clear button is absent while the field is empty. Clicking it removes `q`. |
| `visitors_next_page` | ">" on the bottom bar sends the next `offset` and replaces the rows. "<" sends the previous offset. Page buttons show the first page, the last page, and `current - 2` through `current + 2`, with "..." in a gap. There is no "Далее" under the table. |
| `visitors_printer_and_switches` | The printer dropdown in the navigation lists the shared catalog. Choosing one, and turning on both toggles, leaves those three values set after the table reloads. |
| `visitors_one_search_field` | The screen has a single text field for search. |
| `visitors_opens_a_prefilled_form` | The Посетитель tab is absent until a row is opened. The first click highlights the row and stays on `/visitors`. The second click opens `/visitor?userId=`, shows Посетитель, hides the header printer, and fills the form from that registration. Close returns to `/visitors`, clears the tab, and shows the header printer again. |
| `visitors_keeps_the_visitor_after_reload` | Opening `/visitor?userId=` with no visitor stored still shows Посетитель and fills the form for that id. |
| `visitors_closes_the_tab_on_leave` | Switching from `/visitor` to another tab closes the Посетитель tab. A changed form asks before that switch or close. Staying keeps the form. Continuing leaves and clears the tab. |
| `visitors_saves_and_prints` | Печать with no edits posts only `POST /api/print` and closes the tab. Сохранить posts the visitor and closes the tab. Печать after an edit posts the save and then the print, then closes the tab. |
| `visitors_print_stays_when_save_fails` | A failed save before print stays on `/visitor`, shows the error, and does not print. |
| `visitors_page_selection` | The header checkbox selects and clears the current page. A page change, a search change, or a category change clears those checks. "Выбрать все" is disabled with an empty search and "Все категории". Once a search or category is set, it selects the visible rows and keeps them selected on the next page. A category change clears that selection. |

### Done when

`npm test -- visitors_` passes, and steps 1 through 5 stay green.

## Step 7. Print confirm

[Back to summary](#summary)

The "Печать" dialog on the visitor list. Confirming it opens `/print`.

### Work

1. "Печать" opens a dialog whose heading is "Внимание!" and whose text is "вы уверены что хотите распечатать бейдж?".
2. The dialog lists packet names from `GET /api/packets`. It does not contain a printer select. The printer is the one chosen above the table.
3. "отмена" closes the dialog and stays on `/visitors`. It posts nothing.
4. "ок" posts `POST /api/packets` for the listed packets, then `POST /api/print` with the checked row ids and the selected printer. When "Заказать сертификат" is on, it also posts `POST /api/certificates` with those ids. When "Отметить пришедших" is on, it also sends `PATCH /api/registrations/{id}` with `gotsome` set for each checked id. Then it navigates to `/print`, carrying those ids in order.
5. The dialog has no "Найдено совпадение" and no "объединить" control.

### Test scenarios

| Scenario | Assert |
|---|---|
| `confirm_lists_packets` | "Печать" shows the warning text and the packet names. The dialog has no "Выберите принтер" select. |
| `confirm_cancel_stays` | "отмена" hides the dialog. The URL is still `/visitors`. No print, packet, certificate, or registration request was sent. |
| `confirm_ok_enqueues_and_opens_print` | With one row checked, printer "HP", both switches off, "ок" posts the packets and `POST /api/print` with that id and "HP", then the URL is `/print`. |
| `confirm_switches_add_their_posts` | With both switches on, "ок" also posts `POST /api/certificates` and `PATCH /api/registrations/{id}` with `gotsome` set, then opens `/print`. |
| `confirm_has_no_merge` | The screen has no "объединить" control. |

### Done when

`npm test -- confirm_` passes, and steps 1 through 6 stay green.

## Step 8. Print progress

[Back to summary](#summary)

`8.html`. The ids come from the confirm step. The count comes from the summary.

### Work

1. The screen reads the id list it was opened with. It polls `GET /api/prints/summary?which=current`.
2. The percent is the summary's printed count over the length of that id list. "Печатает сейчас" is the id at that count. "Следующий" is the following id. An empty remainder leaves "Следующий" empty.
3. "Пауза" and "Продолжить" are one control. Pause stops the poll. Continue starts it again. Neither posts.
4. "Остановить" posts `POST /api/printers/cancel` and navigates to `/visitors`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `progress_shows_current_and_next` | Opened with ids `a`, `b`, `c` and a summary printed count of 1, the percent is 33, "Печатает сейчас" shows `b`, and "Следующий" shows `c`. |
| `progress_polls_until_paused` | Before "Пауза", a second summary response updates the percent. After "Пауза", a further summary response is not requested. "Продолжить" requests it again. |
| `progress_stop_cancels` | "Остановить" posts `POST /api/printers/cancel` and the URL is `/visitors`. |

### Done when

`npm test -- progress_` passes, and steps 1 through 7 stay green.

## Step 9. Import

[Back to summary](#summary)

`6.html`. `6-progress.html` is this screen while the upload is in flight. The HTTP plan left the CSV routes out of `/api`. This screen calls the old paths and treats a failure as "nothing was stored".

### Work

1. "База посетителей" opens a dialog with one select per form field from the loaded form document. "Ok" posts that map to `POST /csv_setup`.
2. "Загрузить CSV" opens a dialog titled "Загрузка CSV" with "Выбрать файл". "Ok" posts the file to `POST /uploadcsv`.
3. While either post is in flight, the percent bar replaces the idle file row. When the post finishes, the bar is gone.
4. A response that is not OK leaves the file name on the row and shows the error. The screen does not show a stored-row confirmation.
5. Back points at `/`. `uploadcsv.html` and `printcsv.html` are not routes.

### Test scenarios

| Scenario | Assert |
|---|---|
| `import_maps_columns` | "База посетителей" lists a select for a form field. "Ok" posts `POST /csv_setup` with the chosen column. |
| `import_uploads_the_file` | Choosing a file shows its name. "Ok" posts `POST /uploadcsv`. While the response is pending, the percent bar is shown and the idle file row is not. |
| `import_failure_keeps_the_file` | A 404 from `POST /uploadcsv` hides the bar, keeps the file name, and shows the error. There is no confirmation that a visitor was stored. |
| `import_back_is_the_menu` | Back points at `/`. |

### Done when

`npm test -- import_` passes, and steps 1 through 8 stay green.

## Step 10. Settings

[Back to summary](#summary)

`settings.mustache`, without the clock row.

### Work

1. Two links: "Настройки принтеров" to `/printers`, and "Настройки регистрации" to `/settings/registration`.
2. The screen has no "Установить текущее время на роутере" control.
3. Back points at `/`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `settings_links_the_two_screens` | The two labels are links to `/printers` and `/settings/registration`. |
| `settings_has_no_clock` | There is no control named "Установить текущее время на роутере". |
| `settings_back_is_the_menu` | Back points at `/`. |

### Done when

`npm test -- settings_` passes, and steps 1 through 9 stay green.

## Step 11. Printers

[Back to summary](#summary)

`9.html` and `printersSetup.mustache`. Routing fields are the ones in [printing.md](printing.md): `zone`, `print_cert`, `auto_print`, `no_postscript`.

### Work

1. The heading is "Тест принтеров".
2. "Обновить список принтеров" posts `POST /api/printers/search` and replaces the table with the returned list. "Сбросить очередь печати" posts `POST /api/printers/cancel`.
3. An empty list shows "Нет подключенных принтеров".
4. Each row shows the queue name and edits `zone`, `print_cert` ("Печать сертификата"), `auto_print` ("Автопечать из анкеты"), and `no_postscript` ("Запретить PostScript").
5. "Тест" posts `POST /api/printers/{name}/test` for that row.
6. "Сохранить" patches `PATCH /api/printers/{name}` with that row's `values`, and `PATCH /api/printers/{name}/routing` with the four routing fields. A 200 shows "Ok" on that row.
7. Back points at `/settings`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `printers_lists_queues` | One queue named "HP" shows that name and its zone. An empty list shows "Нет подключенных принтеров" and no row. |
| `printers_refresh_and_cancel` | "Обновить список принтеров" posts `POST /api/printers/search` and shows the new name. "Сбросить очередь печати" posts `POST /api/printers/cancel`. |
| `printers_test_one_queue` | "Тест" on "HP" posts `POST /api/printers/HP/test` and does not post for the other row. |
| `printers_save_writes_values_and_routing` | Changing the zone and "Сохранить" patches `values` and `routing` for that name. `routing` contains the zone, `print_cert`, `auto_print`, and `no_postscript`. The row then shows "Ok". |

### Done when

`npm test -- printers_` passes, and steps 1 through 10 stay green.

## Step 12. Registration settings

[Back to summary](#summary)

`settingsRegistration.mustache`. The six switches are fields of the settings object from the HTTP plan. "Перезапуск системы" stays off this screen.

### Work

1. `GET /api/settings` sets the switches. Each change patches `PATCH /api/settings` with that one field:

| Label | Field |
|---|---|
| Включить печать с ipad анкеты | `print_on_save` |
| Делать ФИО с заглавной буквы | `capitalize_names` |
| Использовать оплату билетов | `show_ticket_status` |
| Загружать штрихкоды | `load_barcodes` |
| Печатать второй бейдж | `double_print` |
| Печатать бейдж траснлитом | `transliterate` |

2. `GET /api/barcodes` renders one row per category: the category name, and the remaining count over the pool size.
3. The row opens "Минимум" and "Добавить". "ок" patches `PATCH /api/barcodes` with those two counts for that category. "отмена" closes the dialog and does not patch.
4. "Перезапуск синхронизации" posts `POST /api/sync`.
5. The screen has no "Перезапуск системы" control.
6. Back points at `/settings`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `regset_loads_the_switches` | A settings object with `double_print` true shows "Печатать второй бейдж" checked and "Загружать штрихкоды" unchecked when `load_barcodes` is false. |
| `regset_patch_is_one_field` | Turning on "Делать ФИО с заглавной буквы" patches `{ "capitalize_names": true }` and no other field. |
| `regset_barcode_min_and_add` | A category row shows its remaining count and pool size. "ок" with minimum 5 and add 10 patches that category. "отмена" sends no patch. |
| `regset_restarts_sync` | "Перезапуск синхронизации" posts `POST /api/sync`. The screen has no "Перезапуск системы" control. |
| `regset_back_is_settings` | Back points at `/settings`. |

### Done when

`npm test -- regset_` passes, and steps 1 through 11 stay green.
