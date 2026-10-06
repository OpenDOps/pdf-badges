# Registration UI

The operator screens around the form, in `registration-form/`. The form engine itself is [form-design.md](form-design.md). The build sequence is [registration-ui-implementation-plan.md](registration-ui-implementation-plan.md). Login and the event choice stay `registration-admin/`. The device routes these screens call are [http-implementation-plan.md](http-implementation-plan.md).

The original pages are the static HTML under `registration/resources/public/` (`2.html` through `9.html`) and the later mustache pages that the desk actually served (`keyAuth`, `operator`, `form`, `db_view`, `settings`, `settings_registration`, `printersSetup`). The numbered files are the screen layouts. The mustache files are the links between them. A fixed menu in the corner of the static files lists every mockup for the designer. That menu is not navigation.

`/register` stays the kiosk from the form design. It is not one of the screens below. It does not link to them, so their modules stay unloaded while a visitor is on the kiosk.

## How pages load

The app is React Router in framework mode (the continuation of Remix). Each screen is its own route module. The root layout does not import those modules. A screen arrives when the operator opens a URL, or when a `Link` to it is on the screen they are looking at.

Every `Link` uses `prefetch="viewport"`. The target module is fetched when that link is in the viewport, and not before. A screen this page does not link to is not fetched. A control that is a submit, not a link, loads the next module at that navigation. The key screen is that case: it has no link, and the menu module loads when the key is accepted.

The form route renders the desk component already written in `registration-form/app/routes/desk.tsx`. The path is `/form`, which is the link on the operator menu. `/desk` redirects there, so the URL the form plan already uses still opens that component. The form's own steps stay engine state. They are not route segments.

Any operator URL opened with no desk cookie redirects to `/key`. `/register` does not. A good key sets that cookie, and from then on every route in the table below can be opened. The menu shows all four tiles for every accepted key. `operator.mustache` hid import and settings unless the key was an admin key. This app does not. A later request that comes back 401, because the cookie is gone, redirects to `/key` again.

## Routes

| Path | Module | Original page | Links on this screen |
|---|---|---|---|
| `/key` | `routes/key.tsx` | `3.html`, `keyAuth.mustache` | none; a good key navigates to `/` |
| `/` | `routes/menu.tsx` | `4.html`, `operator.mustache` | `/form`, `/visitors`, `/import`, `/settings` |
| `/form` | `routes/desk.tsx` | `5.1.html`, `5.1-if-zero.html`, `5.2.html`, `form.mustache` | `/` |
| `/visitor` | `routes/visitor.tsx` | same pages as `/form` | `/`, `/form`, `/visitors` |
| `/visitors` | `routes/visitors.tsx` | `7.html`, `db_view.mustache` | `/print`, `/` |
| `/print` | `routes/print.tsx` | `8.html` | `/visitors` |
| `/import` | `routes/import.tsx` | `6.html`, `6-progress.html` | `/` |
| `/settings` | `routes/settings.tsx` | `settings.mustache` | `/printers`, `/settings/registration` |
| `/printers` | `routes/printers.tsx` | `9.html`, `printersSetup.mustache` | `/settings` |
| `/settings/registration` | `routes/registration-settings.tsx` | `settingsRegistration.mustache` | `/settings` |

Back in the header is a link to the path in that last column. Forward stays disabled, as on the static pages.

## Key

`3.html` and `keyAuth.mustache`. One field, placeholder "Ключ", and "Войти".

Submit posts `POST /api/desk/auth`. A wrong key stays on this screen and the field shows the error. A good key sets the desk cookie and opens the menu. After that cookie is set, the form, the visitor list, print progress, import, settings, and printers are all reachable. This screen does not list, create, or print keys. Those rows are the admin event screen (`2.html`, `controlExpo`).

## Menu

`4.html` and `operator.mustache`. Four tiles:

| Tile | Opens |
|---|---|
| Оператор | `/form` |
| Печать | `/visitors` |
| Загрузить базу | `/import` |
| Настройки | `/settings` |

`4.html` labeled the fourth tile "Настройка принтеров" and put one "Включить модерацию" checkbox on this page. The working menu renamed that tile to "Настройки" and moved the flags onto the registration settings screen. This screen follows that menu.

The page also shows how many local rows are still waiting to sync (`GET /api/sync`) and the registration QR (`GET /api/qr`). The original page printed local addresses from the mustache context. This screen does not add a route for them.

"Обновить" on the original menu is the binary update. [porting.md](porting.md) step 11 shows it when a newer release is waiting.

## Category and printer

The category and the printer are one choice for the whole desk. The form and the visitor list read the same two lists and the same selected values. Ticket status and packets stay on the form.

`GET /api/desk/catalog` returns both lists together. The data is `rev`, `categories`, and `printers`. `Cache-Control` is `no-store`. The route does not read the desk cookie. `GET /api/forms/categories` still returns the raw categories file. The desk selects do not call it, and they do not call `GET /api/printers`. That printers route remains the CUPS list on the printer settings screen.

`categories` always starts with the preset exhibitor: `cat_id` -2, Russian name Экспонент, English name Exhibitor. The rest is the array in `forms/categories.json` for the open event, in file order, without a second -2 row. Each stored item keeps `cat_id` and `name`. A missing file, or a file that is not a JSON array, leaves only the exhibitor. `printers` is one object per printer file, with `name` and `text` both set to the file stem. The names are the stems of `printer_conf/*.json` next to the credential file, plus `pr_*.json` in the event directory with the `pr_` prefix removed. The two sets are joined and sorted. The route does not ask CUPS.

`rev` is a hash of those two lists. A hash of 0 is sent as 1, so 0 means the client has not seen a list yet. The client sends back the `rev` it was given. The hash stays the same for one process run. A restart can give the same files a new `rev`, and the next response then returns at once.

The client long-polls. The first request uses `rev` 0 and the server answers immediately. Later requests are `GET /api/desk/catalog?rev=` with the last `rev`. When that `rev` matches the lists on disk, the handler waits. It re-reads the files about every 200ms. A change in either list answers with the new body. After 25 seconds with no change, the handler answers with the same body so the connection does not stay open. The client starts the next request as soon as a body arrives. The same `rev` after that hold waits a short moment before the next request, so an unchanged answer does not spin. A failed request waits about a second and asks again.

One poll runs for the app. The first screen that shows a select starts it. The last screen that drops the select stops it and aborts the open request. A new `rev` replaces the lists. The category select always has a value once the list is non-empty: the first category, until the operator picks another. If that category leaves the list, the first remaining category is selected. The printer may stay unselected, and a printer that leaves the list is cleared. Both controls are custom dropdowns. A click opens the list, and the current row is highlighted. The field does not accept typing. A click on the field while the list is open closes it. A click on the field while it is already focused and the list is closed opens the list. Moving from the form to the visitor list keeps the selection in memory. The same two ids are written to `localStorage` under `registration-form.desk`. A full reload has no in-memory record, so those ids are restored when they are still in the lists. A stored category that is gone falls back to the first category. A stored printer that is gone is cleared. With nothing stored, the first category is selected and the printer stays empty. The control is `ClosedSelect`. [pickers.md](pickers.md) is how to reuse it, and how country, city, and the phone calling code reuse the same list.

## Form

`5.1.html`, `5.1-if-zero.html`, and `5.2.html` are three states of one screen, and `form.mustache` is the page the menu already linked. This route renders `Desk` from `routes/desk.tsx`. `/visitor` renders that same component for one open visitor. `routes/visitor.tsx` re-exports it. The header, the step, Save, and the packets stay as [form-design.md](form-design.md) specifies. The category select and the printer select are the shared lists in [Category and printer](#category-and-printer). Save writes that category onto the visitor and sends that printer name. This screen does not grow a second form.

`5.1-if-zero.html` is that same screen when the copy count is zero. `5.2.html` is a comment on a question and "Отклонить" beside "Печать". Both are states of the desk component, not routes. A link back to `/` is the only navigation this screen adds, so the menu module is the one fetched while the operator is here.

## Visitors

`7.html` and `db_view.mustache`. The print tile on the menu opens this list. The frame matches the form: the desk navigation on top, the list in the middle, and an action bar fixed to the bottom.

The top bar is home, Форма, and Посетители, with Посетители current. The list does not reserve a blank gap for a Посетитель tab. The shared printer dropdown sits immediately after Посетители, as a printer icon and the select, and the network status fills the rest of the bar. That dropdown is only on this list. Opening a visitor removes it and adds the Посетитель tab, with a close control on its right. Leaving that tab for any other screen closes it. Closing it returns to this list. The visitor form has Сохранить and Печать. Сохранить writes the visitor and closes the tab. Печать writes first only when the form changed since it opened or was last saved, then prints that visitor, and closes the tab after the server accepts the print. If the form changed since it opened or was last saved, that close or switch asks in a dialog whether to continue. Continuing drops the changes. The printer is the custom list from [Category and printer](#category-and-printer), and the same stored choice. The same status sits in the top-right corner on the other operator screens. Its text truncates only when that space is narrower than the words.

The bottom bar is a pager on the left and "Отметить", "Заказать сертификат", and "Печать" on the right. The pager is "<", page numbers, and ">". The first page, the last page, and the pages from two before the current page through two after it are always shown. A gap in that run is "...". The current number is the selected button. "<" is disabled on the first page and ">" when the next page would pass `total`. The first two action buttons are toggles. Their pressed state stays while the table reloads, for the confirm step. "Печать" opens the confirm dialog in the next step.

The filter is one search field and a category multi-select that starts at "Все категории". When the search field has text, a round clear button at its right edge empties it. Its other options are the shared catalog. "Все категории" is selected alone. Choosing another category adds it and clears "Все категории". Choosing a selected category again removes it. Choosing "Все категории" clears the others and closes the list. Selected rows stay highlighted, and the list stays open while other categories are toggled. That filter does not change the category stored for the form. The table columns are a row checkbox, Имя, Фамилия, Компания, Категория, and `(n)` at the right, where `n` is `print_count`. The first click on a row highlights that row. A second click on the highlighted row opens the Посетитель tab at `/visitor?userId={uid}` and fills the form from `GET /api/registrations/{uid}`. Reloading that address keeps the tab open and loads the same visitor. A click on the row checkbox does not highlight the row. The checkbox column header selects or clears every row on the current page. That page selection is cleared when the page, the search text, or the category filter changes. "Выбрать все" selects every record that matches the current search or category, including rows on other pages, and those checks stay selected when the page changes. A search or category change clears them. That button stays disabled while the search is empty and the category is "Все категории". A row with no company shows the email in that cell, grey: the first two letters, then exactly three stars, then `@` and the domain. An empty local part is still three stars. Even and odd rows use two close background colors. A row with `print_count` above zero uses a light amber pair instead. Rows come from `GET /api/registrations` with `q`, `category`, `limit`, and `offset`. The static page also drew separate email, name, surname, and company fields; the working page left a single search box, and this screen keeps that box.

"Печать" asks "вы уверены что хотите распечатать бейдж?" and lists what to hand out (`GET /api/packets`). "ок" marks those packets given (`POST /api/packets`), enqueues the selected rows, and opens `/print`. "отмена" closes the dialog and stays here. `7.html` drew a printer picker inside that dialog ("Выберите принтер", Ok, Отмена). The printer dropdown in the navigation is that choice; the dialog does not ask again.

A "Найдено совпадение" / "объединить" dialog on `db_view.mustache` is the one-off duplicate merge. It stays out, with the HTTP plan.

## Print

`8.html`. Opened from the visitors screen after the operator confirms a print.

A percent bar, a block titled "Печатает сейчас", and a block titled "Следующий". The lines are the running job and the next waiting job from `GET /api/prints/summary`, refreshed while this screen is open.

"Пауза" and "Продолжить" are the two states of one control, as on the static page: pause holds the next job on this screen, continue lets it go. "Остановить" posts `POST /api/printers/cancel` and returns to `/visitors`.

## Import

`6.html`, with `6-progress.html` as the same screen while a file is uploading. The menu links here for every accepted key. The back link is `/`.

Two dialogs, matching the two tabs on `6.html`:

| Tab | Dialog |
|---|---|
| База посетителей | one select per form field, so a CSV column can be mapped, then "Ok" |
| Загрузить CSV | "Выбрать файл", then "Ok" |

While the file is uploading, the percent bar from `6-progress.html` replaces the idle file row. That bar is this screen, not a second route.

The HTTP plan left `POST /uploadcsv`, `POST /set_separator`, `POST /csv_setup`, and `POST /remove_last_csv` out of this port. This screen is the page those calls belonged to. It does not invent a replacement importer. The file controls call those paths when they exist, and until then the screen shows the file row and does not pretend a row was stored.

`uploadcsv.html` and `printcsv.html` are titled "Upload CSV" and are not a second import screen.

## Settings

`settings.mustache`. The menu links here for every accepted key. Two links:

| Link | Opens |
|---|---|
| Настройки принтеров | `/printers` |
| Настройки регистрации | `/settings/registration` |

"Установить текущее время на роутере" called `GET /setdate`. Setting the device clock stays out, with the HTTP plan, so this screen does not show that row.

## Printers

`9.html` and `printersSetup.mustache`. Title "Тест принтеров".

"Обновить список принтеров" posts `POST /api/printers/search` and replaces the table. "Сбросить очередь печати" posts `POST /api/printers/cancel`. An empty list says "Нет подключенных принтеров".

Each row is one queue from `GET /api/printers`. The columns are the queue name, "Зона", "Печать сертификата", "Автопечать из анкеты", and "Запретить PostScript". "Тест" on a row posts `POST /api/printers/{name}/test`. "Сохранить" writes that row's values with `PATCH /api/printers/{name}` and its routing with `PATCH /api/printers/{name}/routing`. A saved row confirms with "Ok".

Choosing a printer for one visitor stays on the form screen. This screen is where the queue, the zone, and the routing are stored.

## Registration settings

`settingsRegistration.mustache`. Six switches, posted as one object to `PATCH /api/settings` when any of them changes:

| Label | Flag |
|---|---|
| Включить печать с ipad анкеты | print from the iPad form |
| Делать ФИО с заглавной буквы | capitalize names |
| Использовать оплату билетов | show ticket status |
| Загружать штрихкоды | load barcodes |
| Печатать второй бейдж | print a second badge |
| Печатать бейдж траснлитом | print the badge in transliteration |

Under the switches, one row per category from `GET /api/barcodes`: the category name and a bar of remaining codes over the pool size. The row opens "Минимум" and "Добавить". "ок" posts those two counts with `PATCH /api/barcodes`. "отмена" closes the dialog.

"Перезапуск синхронизации" posts `POST /api/sync`. "Перезапуск системы" called `GET /reload`, which the HTTP plan describes as an empty 200 that reloads nothing. This screen does not show that button.

## Pages that stay out

| Original | Why it is not a screen here |
|---|---|
| `admin_login_2.html`, `GET /admin` | Login is the admin app. |
| `2.html`, `controlExpo.mustache`, `exposlist.mustache` | The event list, and the keys edited on that page, stay the admin app. |
| `index.html` | A stub that prints `ctrlscr`. |
| `10.html` | A checkbox widget demo, not a desk page. |
| `test_form.mustache` | Duplicate search, merge, and a memory backup. Left out of the HTTP port. |
| `uploadbarcodes.html`, `uploadvisitcodes.html` | A hand upload of barcode files. Sync already downloads them. |
| Self-update on the operator menu | [porting.md](porting.md) step 11, when a newer release is waiting. |
