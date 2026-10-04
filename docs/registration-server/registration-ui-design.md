# Registration UI

The operator screens around the form, in `registration-form/`. The form engine itself is [form-design.md](form-design.md). Login and the event choice stay `registration-admin/`. The device routes these screens call are [http-implementation-plan.md](http-implementation-plan.md).

The original pages are the static HTML under `registration/resources/public/` (`2.html` through `9.html`) and the later mustache pages that the desk actually served (`keyAuth`, `operator`, `form`, `db_view`, `settings`, `settings_registration`, `printersSetup`). The numbered files are the screen layouts. The mustache files are the links between them. A fixed menu in the corner of the static files lists every mockup for the designer. That menu is not navigation.

`/register` stays the kiosk from the form design. It is not one of the screens below. It does not link to them, so their modules stay unloaded while a visitor is on the kiosk.

## How pages load

The app is React Router in framework mode (the continuation of Remix). Each screen is its own route module. The root layout does not import those modules. A screen arrives when the operator opens a URL, or when a `Link` to it is on the screen they are looking at.

Every `Link` uses `prefetch="viewport"`. The target module is fetched when that link is in the viewport, and not before. A screen this page does not link to is not fetched. A control that is a submit, not a link, loads the next module at that navigation. The key screen is that case: it has no link, and the menu module loads when the key is accepted.

The form route renders the desk component already written in `registration-form/app/routes/desk.tsx`. The path is `/form`, which is the link on the operator menu. `/desk` redirects there, so the URL the form plan already uses still opens that component. The form's own steps stay engine state. They are not route segments.

A desk cookie is required on every screen except `/register` and `/key`. A missing cookie, or a 401 from a desk route, opens `/key`. The menu draws the import link and the settings link only when the accepted key is an admin key, which is how `operator.mustache` hid those two items. Their modules load only in that case.

## Routes

| Path | Module | Original page | Links on this screen |
|---|---|---|---|
| `/key` | `routes/key.tsx` | `3.html`, `keyAuth.mustache` | none; a good key navigates to `/` |
| `/` | `routes/menu.tsx` | `4.html`, `operator.mustache` | `/form`, `/visitors`; an admin key also links `/import` and `/settings` |
| `/form` | `routes/desk.tsx` | `5.1.html`, `5.1-if-zero.html`, `5.2.html`, `form.mustache` | `/` |
| `/visitors` | `routes/visitors.tsx` | `7.html`, `db_view.mustache` | `/print`, `/` |
| `/print` | `routes/print.tsx` | `8.html` | `/visitors` |
| `/import` | `routes/import.tsx` | `6.html`, `6-progress.html` | `/` |
| `/settings` | `routes/settings.tsx` | `settings.mustache` | `/printers`, `/settings/registration` |
| `/printers` | `routes/printers.tsx` | `9.html`, `printersSetup.mustache` | `/settings` |
| `/settings/registration` | `routes/registration-settings.tsx` | `settingsRegistration.mustache` | `/settings` |

Back in the header is a link to the path in that last column. Forward stays disabled, as on the static pages.

## Key

`3.html` and `keyAuth.mustache`. One field, placeholder "Ключ", and "Войти".

Submit posts `POST /api/desk/auth`. A wrong key stays on this screen and the field shows the error. A good key sets the desk cookie and opens the menu. This screen does not list, create, or print keys. Those rows are the admin event screen (`2.html`, `controlExpo`).

## Menu

`4.html` and `operator.mustache`. Four tiles:

| Tile | Opens |
|---|---|
| Оператор | `/form` |
| Печать | `/visitors` |
| Загрузить базу | `/import`, admin key only |
| Настройки | `/settings`, admin key only |

`4.html` labeled the fourth tile "Настройка принтеров" and put one "Включить модерацию" checkbox on this page. The working menu renamed that tile to "Настройки" and moved the flags onto the registration settings screen. This screen follows that menu.

The page also shows how many local rows are still waiting to sync (`GET /api/sync`) and the registration QR (`GET /api/qr`). The original page printed local addresses from the mustache context. This screen does not add a route for them.

"Обновить" on the original menu is the binary self-update. That stays out, with the HTTP plan.

## Form

`5.1.html`, `5.1-if-zero.html`, and `5.2.html` are three states of one screen, and `form.mustache` is the page the menu already linked. This route renders `Desk` from `routes/desk.tsx`. The header, the step, Save, the printer chosen for this save, and the packets stay as [form-design.md](form-design.md) specifies. This screen does not grow a second form.

`5.1-if-zero.html` is that same screen when the copy count is zero. `5.2.html` is a comment on a question and "Отклонить" beside "Печать". Both are states of the desk component, not routes. A link back to `/` is the only navigation this screen adds, so the menu module is the one fetched while the operator is here.

## Visitors

`7.html` and `db_view.mustache`. The print tile on the menu opens this list.

Two switches sit above the table: "Отметить пришедших" and "Заказать сертификат". A printer select is labeled "Используемый принтер" and is filled from `GET /api/printers`. The choice is remembered for the next print on this screen.

The filter is one search field and a category select that starts at "Все категории". The table columns are a row checkbox, E-mail, Имя, Фамилия, Компания, and Категория. Rows come from `GET /api/registrations` with `q`, `category`, `limit`, and `offset`. The static page also drew separate email, name, surname, and company fields; the working page left a single search box, and this screen keeps that box. Pages sit under the table.

"Печать" asks "вы уверены что хотите распечатать бейдж?" and lists what to hand out (`GET /api/packets`). "ок" marks those packets given (`POST /api/packets`), enqueues the selected rows, and opens `/print`. "отмена" closes the dialog and stays here. `7.html` drew a printer picker inside that dialog ("Выберите принтер", Ok, Отмена). The printer select above the table is that choice; the dialog does not ask again.

A "Найдено совпадение" / "объединить" dialog on `db_view.mustache` is the one-off duplicate merge. It stays out, with the HTTP plan.

## Print

`8.html`. Opened from the visitors screen after the operator confirms a print.

A percent bar, a block titled "Печатает сейчас", and a block titled "Следующий". The lines are the running job and the next waiting job from `GET /api/prints/summary`, refreshed while this screen is open.

"Пауза" and "Продолжить" are the two states of one control, as on the static page: pause holds the next job on this screen, continue lets it go. "Остановить" posts `POST /api/printers/cancel` and returns to `/visitors`.

## Import

`6.html`, with `6-progress.html` as the same screen while a file is uploading. The menu links here only for an admin key. The back link is `/`.

Two dialogs, matching the two tabs on `6.html`:

| Tab | Dialog |
|---|---|
| База посетителей | one select per form field, so a CSV column can be mapped, then "Ok" |
| Загрузить CSV | "Выбрать файл", then "Ok" |

While the file is uploading, the percent bar from `6-progress.html` replaces the idle file row. That bar is this screen, not a second route.

The HTTP plan left `POST /uploadcsv`, `POST /set_separator`, `POST /csv_setup`, and `POST /remove_last_csv` out of this port. This screen is the page those calls belonged to. It does not invent a replacement importer. The file controls call those paths when they exist, and until then the screen shows the file row and does not pretend a row was stored.

`uploadcsv.html` and `printcsv.html` are titled "Upload CSV" and are not a second import screen.

## Settings

`settings.mustache`. The menu links here only for an admin key. Two links:

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
| Self-update on the operator menu | The device image owns that. |
