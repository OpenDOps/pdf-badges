# Registration form

How the quest config becomes a multipage form, and how the same pages run inside four layouts. The build sequence is [form-implementation-plan.md](form-implementation-plan.md). The device routes are [http-implementation-plan.md](http-implementation-plan.md). Login and the event choice stay the admin app.

The Angular source is `form-app`. `operator-form` and `nexporeg-form` both call `RegService.getRegData`. The iPad HTML, the wide web HTML, and the phone CSS are three skins on `reg-screens`. The operator page is a fourth skin: search, category, printer, and packets around that same screen walker. The React form keeps that split. Components and the page engine are one. A layout only changes the frame and the spacing.

## What is loaded

For one form the client loads `GET /forms/form.json` once. Vite reads the source files and responds with that document: vars, the structure, the empty model, `conf`, `hiddenq`, settings, and barcode pools. `jv_rights` and `*_jv_rights` are omitted from the response. The source files stay unchanged. Country, region, and city lists stay the dbenum files. Categories for the operator frame stay `GET /api/forms/categories`.

| part | role |
|---|---|
| vars | named lists. An array is a fixed questionnaire. An object with `name` and `depends` is a place list |
| structure | the shape of `UniRegUser` |
| model | one empty visitor, copied into the working state |
| `conf`, `hiddenq` | the screens and the hidden questions |
| settings, barcodes | form settings and barcode pool sizes |

`conf` is an array of screens, in step order. A screen is:

| field | meaning |
|---|---|
| `advice`, `advice_tip` | the screen title and the parenthetical, each `{ ru, en }` |
| `body` | questions, in order. An item is one question, or an array of questions drawn on one row |
| `for_locales` | the screen is skipped when the current locale is absent. Empty means every locale |
| `screen_block` | screens that share a number are one step and are shown together |
| `next` | rules `{ step, conditions }`. The first rule that passes is the following step. A step of `-1` ends the walk |
| `forhiddens` | this screen may show questions from `hiddenq` |

A question in `body` or in `hiddenq` is:

| field | meaning |
|---|---|
| `id` | dotted path into the visitor. This is also the question's name in conditions |
| `ru`, `en` | the label |
| `type` | `input`, `select`, `chboxes`, or `tree` |
| `sub` | for `input`: `text`, `email`, `tel`, `mydate`. `bigtxt` uses a textarea |
| `req`, `req_loc`, `req_index` | required for every locale, required for one locale, or "at least one question in this index" |
| `other`, `other_req`, `binded_other` | a free-text line on a choice, and whether that line is required |
| `single_select` | `chboxes` stores one value |
| `dbenum_def`, `dbenum_defs` | the option id chosen when the value is still empty |
| `unique_num` | email or phone must be unique up to this count |
| `dontchange` | warn once before editing a person loaded from search |
| `disabled` | the control is shown and does not accept input |

`hiddenq` questions are built the same way and marked hidden. They stay in the question map so a condition can read them. A screen shows one when its body names that id.

## How the form is built

`build(formId, locale)` returns the engine state. It does not choose a layout.

1. Load `form.json`.
2. Walk the structure into leaves. A leaf is `single` or repeating, a parametric string (`possible_links.str`), an enum (`jvenum_field`, var name `av_p_vars.…` with that prefix removed), or an option field (`jvopt_field`, key `opt_N`). A repeating leaf keeps `rel_prop_unique` and `_filter_fields_` (a phone list filtered to `phone_type` 2 is the mobile list).
3. Copy the empty model. That object is the visitor that will be posted.
4. Map each screen. For each body item, `bind(question)` finds the leaf at `id` and stores the question under that id. The same id in two screens is one question. A body item that is an array becomes one row.
5. Bind `hiddenq` the same way, with `hidden` set.
6. Call `options(question)` once per id. A fixed list resolves immediately. The country list loads once for the locale. Region and city lists load later, when that address has the parent ids.
7. The first step is the first screen whose `for_locales` contains `locale`, or the first screen when `for_locales` is empty. Screens that share its `screen_block` are included in that step.

```text
conf[]
  screen
    body[]  ── question id ──► leaf in the structure ──► path in the visitor
    next[]  ── conditions read question values
```

`setValue(id, locale, value)` writes through the leaf:

| leaf | write |
|---|---|
| parametric string | `node[locale].str` |
| enum | `node.e_val` |
| option field `opt_N` | one id, or a list of ids when the question is `chboxes` and not `single_select` |
| repeating | the primary link (`jvrel_prop.primary`). A new row is appended when the control adds one |

A place or a phone does not scan the question map for a name that shares an id prefix. Those fields read the address object they were bound to. That link is the next section.

## Pages

One step is on screen. Next does not advance until every question on the step passes `findQuestionErrors` for the locale: a required empty value, a required group where none of the `req_index` questions has a value, a required "other" line, a phone with no national digits, an email or phone over `unique_num`. The first error is the one the layout scrolls to.

When the step is valid, `nextStep(screen, questions, locale)` walks `next` in order. A rule with no `conditions` passes. Otherwise `conditions.operator` is `or` or `and` (anything other than `or` is `and`). Each entry in `expression` is `{ q_id, value }`. A list value passes when it contains `value`. A scalar passes when it is `=== value`. The first passing rule's `step` is the next screen index. The walk remembers the steps taken. Back pops one step and does not re-check.

A `screen_block` step shows every screen in that block, in conf order, as one page. Next uses the `next` rules of the last screen in the block.

`for_locales` is applied when the locale changes. A step that does not include the new locale is replaced by the nearest later step that does. Answers already stored stay in the visitor.

The end step is `step === -1`. The frame shows where to collect the badge, then `/register` returns to the greeting. Post is the save action of the frame that is using the engine.

## Address

Country, region, city, and the phones on one address are one value. The Angular form finds them by scanning every question id for a shared string prefix, and treats "this form has a country var" as "this phone has a country". That scan is what this section replaces.

An address is the object the structure already stores under a path such as `personalData.companies.addresses`. Bind walks that object once, at build:

| field on the address | var name | parents |
|---|---|---|
| `country_db` | `country` | none |
| region | `region` | `country` |
| city | `city` | `country`, then `region` |
| `phones_faxes`, `contact_phones`, `faxes` | — | the country on this same object |

A second company address is a second object. A write on the first does not read or clear the second. The link is the object, not `id.indexOf(prefix) == 0`. A question id with no dot used to take the whole form as its prefix, because `substring(0, -1)` is the empty string, and every question id starts with the empty string.

Empty is `null`. The stored enum uses `e_val: null`. The Angular client wrote `-1` for a missing country and then treated both `undefined` and any negative number as empty, so a real id `0` and a missing value took different branches. `null` is the only empty. A country that is `null` leaves region and city unset and does not request their lists.

### Place lists

`PlaceSelect` is the control for country, region, and city. One choice. The label is `v[locale]`. The stored value is the option id.

Lists are cached by `(name, parent ids, locale)`.

| list | request | when |
|---|---|---|
| countries | `GET /api/enums/{locale}/country` | once per locale, at build |
| regions | `GET /api/enums/{locale}/region?country=` | the address country is set |
| cities | `GET /api/enums/{locale}/city?country=&region=` | the address country and region are both set |

A second ask for the same key returns the cached list. Two controls that ask the same key at the same time share one request. A failed request sets the error on that control and leaves the previous list. It does not poll. The Angular loader waited 200 ms in a loop until a cache slot appeared, and a rejected request never filled the slot.

The country list is the only place list loaded at build. Regions load when the country is set. City files for that country then start with the default region, and the other regions follow. City files and per-region phone-mask files share one queue of 10. A region already loaded is not requested again. As each city file arrives, its rows join a local index. The city field searches that index by the typed string. The Angular `load_all` path walked every country and every region and joined the city lists for all of them.

Changing country on an address sets that address's region and city to the default row of the variable list just loaded (`d: true`, otherwise the question default when that id is in the list, otherwise the only row). It does not write `null`. Region is not a UI field. Choosing a city writes that city and the region whose list contains it, and does not write the country. `dbenum_def` applies only when the value is `null` and that id is in the list just loaded.

`load_all` no longer writes parent ids onto sibling questions by scanning `depends_on_names`. The parents are the fields of this address.

### Phones on that address

`PhoneList` is the control for a repeating phone leaf on the address (`phones_faxes`, `contact_phones`, `faxes`). The leaf's `_filter_fields_` (`phone_type`) decides which rows this control shows, so mobile and fax stay separate lists. Each row is one link. The primary row is `jvrel_prop.primary`. Add appends a link with `jvlink_id` `-1` and `primary` false. Remove deletes that link. The last row stays. Marking primary clears `primary` on the other rows of this list.

The calling code and the default mask come from `country_phone_code.json` and `country_phone_mask.json`. Selecting a calling code on the phone loads that country's row. The address country does not choose it. The mask cell can hold several patterns, separated by `; `, and the area code is in parentheses. Russia is `+7 (ddd) ddd-dd-dd`. The masks applied while typing are stored per region, in `country-{countryId}/region-{regionId}/city_phone_mask.json`. Selecting a calling code applies the country pattern, requests that country's default region first, and loads the other region files in the background through the same queue of 10 as the city files. Each file that arrives reapplies the mask. Switching region uses the file when it has arrived, and fetches that region's file when it has not started. The field uses the stored mask once that city code is fully typed. Until then it keeps the country pattern, and it chooses among a country's patterns by the leading digits of the stored codes. Berezino's row is `+375 (1715) d-dd-dd`. A five-digit code such as Arzamas is `+7 (83147) d-dd-dd` in that city's row. Deleting back out of the code restores the country pattern. The field does not name the city or the operator. A city or operator image on the phone would come from the digits, and it does not follow the address region or city. The form does not ship libphonenumber. The place masks were filled from its geocoding data, and the operator lines from its carrier data.

No calling code on the phone means the national part is a plain digit field and the calling code is empty. The control does not pick the first country in the form, which is what `getQuestionsByDBEnumName('country')[0]` did when this address had no country. An address can be one country while the phone keeps another. Changing the address loads phone masks in the background while that phone has no calling code of its own and no number. An entered number stays, with its calling code. Selecting a calling code keeps the national digits and replaces the prefix, on this phone only. The Angular `updatePhoneMask` incremented a counter on every question whose form merely contained some country var, so one address remasked every phone on the page.

The stored value is `str_number`: the calling code plus the national digits, with the mask characters removed. `country_code`, `city_or_operator`, and `number` are not written. The Angular checker had empty branches for those three fields, so they could disagree with `str_number` and still pass. A value with no national digits is empty. The national part is what remains after the calling code loaded for that address, so `+7` is empty and `+7123` is not. The old check treated the string `"+7"` as empty and any other leftover mask as filled.

`unique_num` greater than zero calls `GET /api/registrations/counts?phone=` with that `str_number` when the step is checked. A count at or above `unique_num` sets the error on that question.

## Emails

An email question is `sub` `email`, or its id ends in `emails.email` (`personalData.emails.email`, `personalData.companies.addresses.emails.email`). `EmailList` is the control. The leaf is the repeating `emails` list. One row is `{ email, jvrel_prop.primary }`. Add, remove, and primary work as they do for phones.

The step check rejects a value that does not contain `@` with a non-empty local part and a dot in the domain. `unique_num` calls `GET /api/registrations/counts?email=`. The form setting that allows one person per email is that count: above the limit, the error says the address is already used.

When search filled this visitor and the question is `dontchange`, the first edit opens a warning once and does not write. An email question says this changes the person already registered on that email. Any other question says this changes the person already loaded from search. After the warning is dismissed the field stays editable. `disabled` is the stronger flag and never edits.

`req_index` shared by the personal email and the company email means one of them is enough.

## Questionnaire selects

A questionnaire question is one whose leaf key is `opt_N`, or whose `type` is `select`, `chboxes`, or `tree` and whose var is an array. The array items are `{ id, v: { ru, en }, default, opts }`. `opts` is the children of a tree node.

`ChoiceList` is the control.

| type | what is stored | what is drawn |
|---|---|---|
| `select` | one id on `e_val` or on the option field | a searchable list, one choice. `other` adds a text line under it. `binded_other` writes that text to the sibling question instead of a local string |
| `chboxes` with `single_select` | one id | a radio row per option |
| `chboxes` | a list of ids | a checkbox per option. An option may have an extra input (`setChbInput`) |
| `tree` | one id, or a list when not `single_select` | the same choices, indented by `opts` |

`other_req` fails the step when "other" is selected and the text line is empty. A checkbox option with a required extra input fails the same way. `dbenum_def` / `dbenum_defs` apply when the stored value is empty: the matching option id is written once, at build.

Sort is by `priority` (a default option is `0`) then by the locale label. `create` on a select is the "other" path. The visitor cannot invent an id that is not in the list unless `other` is set.

## Layouts

The engine exposes `step`, `back`, `next`, `questions`, and `setValue`. Four layouts render that. Picking a layout does not rebuild the form. Spacing is Tailwind utilities on the frames. `app/index.css` imports Tailwind. A size the default scale does not have is a `@theme` token in that file (`max-w-kiosk`, `max-w-web`, `pb-bar`).

| layout | when | frame |
|---|---|---|
| `ipad` | a touch device whose short side is at least 768 | one column, the advice as the title, the questions centered, Back and Next as the bottom bar. A body row of several questions is one horizontal row |
| `web` | a pointer device whose width is at least 768 | the same step, a narrower column, the advice as a heading, Back and Next under the questions |
| `phone` | `GET /register` with `Sec-CH-UA-Mobile: ?1` | one question stack, full width, labels above the controls, Back and Next fixed to the bottom. A body row becomes a vertical stack. Place, phone, and email lists use the full width |
| `operator` | the route `/desk` | the operator page. A sticky header holds category, a paid/unpaid pair, printer, and packets. Save is fixed to the bottom. |

`/register` is one URL and two layouts. The loader reads `Sec-CH-UA-Mobile`. `?1` serves the phone layout. A missing header or `?0` serves the kiosk layout, and the response varies on that header. Inside the kiosk layout, `ipad` and `web` follow the viewport, including a resize. A resize does not swap the phone layout for the kiosk layout. `operator` is the route `/desk`. On a narrow window the operator page uses the phone spacing inside its main column and keeps the operator fields above the step.

The operator page lists every reachable screen on one page. From a screen, every passing `next` rule opens its own path, and that path continues through the screens it reaches. A path such as B, D, E, F stays visible together when B's condition passes, and a sibling path C, H, F stays hidden. When no condition from the parent passes, none of those blocks are shown. A screen that both paths reach, such as F, is shown once for whichever path is open. Next and Back are not on the desk. Scalar fields sit three across from 865px, and a checkbox or radio list uses that same three-column grid. Category, the paid/unpaid pair, printer, and packets sit in a sticky header. Save is fixed to the bottom of the viewport. Category and ticket status are written onto the visitor as `category` and `subscribtion.jvrel_prop.ticket_status`, and passed to `POST /api/registrations` with the printer name. Packets edit `ext_packets` on the same object.

`/register` opens on a greeting: the locale control and a start button. Start shows the first step. `form-app` draws that greeting in `start-screen`.

Save on every layout is the same post: the visitor object, after the current step validates. On `/desk` that check covers every screen the conditions leave visible. On `/register`, Next that lands on `-1` posts when `print_on_save` is set. The screen then shows `Получите Ваш бейдж на стойке выдачи бейджей.`, or `Take your badge at the registration desk.`. A `ticketStatus` other than 1 shows the pay line instead: `Вы зарегистрированы. Пожалуйста оплатите участие на кассе.` / `You are registered. Please pay for your participation at the ticket desk.` A non-empty `zone_name` from `save` replaces the badge line. `zone_name` is the printer sector `POST /datapost` returned as `zoneName.ru`. `form-app` leaves that field unread (`zones` is a TODO). After 15 seconds the greeting is back and the model is empty, so the next person starts clear. On `/desk`, Save is a button on the page and posts the same way, including the printer. The desk does not show the greeting. `no_ticket_code`, `no_printer`, and `invalid_phone_email_combo` are shown on the step that is open and are not queued.

A `/register` save that fails before a response, or with a 5xx, appends the visitor to `localStorage` (`registration-form.outbox`) and returns to the greeting. The badge line stays off. The queue retries one visitor at a time, on load and every 5 seconds, and removes it when `save` succeeds. The next registration can proceed while a retry is still failing. `form-app` retries 5 times on the open screen and then alerts. `/desk` does not use the queue.

Idle on `/register` is two minutes from the last pointer or key, unless the query has `reset` seconds. Idle restores the empty model and the greeting. `/desk` does not idle-reset.

## What this form does not do

- Fetch HTML from `assets/d/ipad` or `assets/d/tyumen`, or compile a template.
- Read the visitor, the printers, or the category from `window`.
- Draw a different component tree per layout. The four frames share `Step`, `QuestionField`, `PlaceSelect`, `PhoneList`, `EmailList`, and `ChoiceList`.
- Call the hosted shop. A next rule whose step the config marks as payment is treated as `-1`.
- Long-poll `/was_scanned`.
- Own the desk key, the operator menu, the visitor list, or the printer-routing screen. Those are the other routes in [registration-ui-design.md](registration-ui-design.md). This form only chooses a printer for this save.
