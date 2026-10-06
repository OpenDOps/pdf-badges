# Registration form — step 10

Build sequence for [porting.md](porting.md) step 10. The screens, the page engine, and the address link are [form-design.md](form-design.md).

This plan adds `registration-form/`, its own React Router app. `/register` is the kiosk. `/desk` is the operator form. Both render the same step engine. The app does not import `registration-admin`. Login and the event list stay that app. The desk key, the menu, the visitor list, print progress, settings, and printers are [registration-ui-design.md](registration-ui-design.md). This plan is the form engine.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 10 is done when steps 1 through 15 are done. Steps 8 and 9 of the port can still be open. This plan does not start the registration server.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. App](#step-1-app) | React Router project, `/register` (kiosk or phone) and `/desk` | done |
| [2. Client](#step-2-client) | The form JSON and the dbenum lists Vite serves | done |
| [3. Leaves](#step-3-leaves) | The structure walk | done |
| [4. Values](#step-4-values) | Bind a question and write the visitor | done |
| [5. Walk](#step-5-walk) | Next, Back, a screen block, locales | done |
| [6. Check](#step-6-check) | Required fields before the step advances | done |
| [7. Place](#step-7-place) | Country, region, and city on one address | done |
| [8. Phones](#step-8-phones) | Country masks from JSON | done |
| [9. Emails](#step-9-emails) | The email list, uniqueness, and the one-time warning | done |
| [10. Choices](#step-10-choices) | Select, checkboxes, and a tree | done |
| [11. Step](#step-11-step) | One step rendered with the shared controls | done |
| [12. Layouts](#step-12-layouts) | iPad and web from the viewport, on the kiosk layout | done |
| [13. Register](#step-13-register) | The kiosk route: save, idle, save errors | done |
| [14. Kiosk finish](#step-14-kiosk-finish) | Greeting, the badge line or the zone, and the retry queue | done |
| [15. Desk](#step-15-desk) | The operator route: search, category, save | done |

Shared rules:

- The app is `registration-form/`. React Router runs in framework mode (the continuation of Remix): file routes and loaders. A form step is engine state. It is not a path segment.
- `/register` has two layouts. The loader reads `Sec-CH-UA-Mobile`. `?1` serves `layouts/phone.tsx`. A missing header or `?0` serves `layouts/kiosk.tsx`. The response sets `Vary: Sec-CH-UA-Mobile`.
- Question labels come from the quest config (`ru`, `en`). Chrome strings (Back, Next, Save, the error sentences) come from `app/locales/{ru,en}.json` through `react-i18next`. A missing chrome key fails the test run. The server's error code is mapped with `t("error." + code)`.
- `FormClient` is the only module that knows URLs. It fetches the files Vite serves from `public/`. Tests do not start `rust-reg` and do not call cupsd. When step 9 of the port exists, the same interface calls `/api`.
- Engine tests are plain Vitest. A test that renders uses Testing Library and jsdom. The command is `npm test` in `registration-form/`. The filter is the name in **Done when**.
- Empty enum storage is `e_val: null`. A place or a phone reads the address object it was bound to.
- Frames are Tailwind utilities. `@tailwindcss/vite` and `tailwindcss` match the admin app. `app/index.css` is `@import "tailwindcss"`. A size the default scale does not have is a `@theme` token in that file, used as a utility (`max-w-web`, `pb-bar`). Layouts do not add a second stylesheet.

```text
registration-form/
  package.json
  react-router.config.ts
  app/
    root.tsx
    routes.ts
    routes/register.tsx
    routes/desk.tsx
    layouts/kiosk.tsx
    layouts/phone.tsx
    form/client.ts
    form/engine.ts
    components/Step.tsx
    index.css                  `@import "tailwindcss"` and `@theme` tokens
    locales/ru.json
    locales/en.json
  public/
    forms/                     source JSON: user_vars, user_struct, user_model, quest_conf, glob_stngs, totalbarcodes
    dbenums/                   country, region, and city lists, including phone code and mask
```

## Step 1. App

[Back to summary](#summary)

The project exists and the two routes render. No form engine yet.

### Work

1. Add `react-router` and `@react-router/dev`, React 19, TypeScript, Vitest, Testing Library, and `react-i18next` to `registration-form`. Keep `public/` and the Vite server that already serves it.
2. `routes.ts` declares `/register` and `/desk`. `/desk` renders the operator frame title from the catalog.
3. The `/register` loader reads `Sec-CH-UA-Mobile` and renders one of two layouts. `?1` renders `layouts/phone.tsx`. A missing header or `?0` renders `layouts/kiosk.tsx`. The response sets `Vary: Sec-CH-UA-Mobile`. Both layouts show their frame title from the catalog.
4. Any other path renders the root error, not a form.

### Test scenarios

| Scenario | Assert |
|---|---|
| `routes_register_is_the_kiosk` | `GET /register` with no `Sec-CH-UA-Mobile`, or with `?0`, shows the kiosk frame title. The same path with `Sec-CH-UA-Mobile: ?1` shows the phone frame title. |
| `routes_desk_is_the_operator` | Loading `/desk` shows the operator frame title. |
| `routes_unknown_path_is_not_a_form` | A path other than those two shows the error title. |

### Done when

`npm test -- routes_` passes.

## Step 2. Client

[Back to summary](#summary)

The form JSON and the enum lists load through one client. Vite serves `public/` at the site root. `GET /forms/form.json` is answered by the Vite plugin, which reads the source files and returns the folded document. `/register` and `/desk` load that document and show the first screen advice.

### Work

1. `FormClient` has `load()` for the form documents; `enums(locale, name, parents)`; `counts(kind, value)`; `search(text)`; `save(visitor, printer)`; `photo(id, bytes)`; `printers()`.
2. `load` fetches `/forms/form.json` once. Vite reads `user_vars.json`, `user_struct.json`, `user_model.json`, `quest_conf.json`, `glob_stngs.json`, and `totalbarcodes.json` and responds with one document: vars, structure, model, `conf`, `hiddenq`, settings, and barcodes. Every `jv_rights` and `*_jv_rights` key is omitted from that response. The source files stay unchanged.

3. `enums` fetches the dbenum file for that key:

| list | path |
|---|---|
| country | `/dbenums/country_{locale}.json` |
| region | `/dbenums/country-{countryId}/region_{locale}.json` |
| city | `/dbenums/country-{countryId}/region-{regionId}/city_{locale}.json` |
| phone code | `/dbenums/country_phone_code.json` |
| phone mask | `/dbenums/country_phone_mask.json` |
| phone carrier | `/dbenums/country_phone_carrier.json` |

Step 8 loads the country code and the default country mask when the phone calling code is selected. A country can have several masks in that row. The area code sits in parentheses, as in `+7 (ddd) ddd-dd-dd`. The masks the field applies while typing live in one file per region, `/dbenums/country-{countryId}/region-{regionId}/city_phone_mask.json`. Selecting a country applies the country pattern, requests the current region first, and loads the other region files in the background through the same queue of 10 as the city files. Each file that arrives reapplies the mask. Switching region uses the file if it has arrived, and fetches that region's file if it has not started. The field uses the stored mask from the loaded file once that city code is fully typed. Until then it keeps the country pattern, and it chooses among a country's patterns by the leading digits of the stored codes. `+375 (1715) d-dd-dd` is Berezino's row. A five-digit code such as Arzamas is `+7 (83147) d-dd-dd` in that city's row. Deleting back out of the code restores the country pattern.

4. Enum results are cached by `(name, parent ids, locale)`. `enumPath` accepts only locales `ru` and `en`, and only integer parent ids. Two callers that ask the same key while the first request is open share that request's result, including the error when a refresh fails and a list is already cached. A later ask receives a shallow copy of the cached list. A failed `counts` or `search` error names the route and the status, without the query.

The copied model stores an empty region and city as `e_val: -1`, and country `219` (Россия). Region `3948` and city `17849` are Москва. Later steps load those ids from these files.

### Test scenarios

| Scenario | Assert |
|---|---|
| `documents_load_returns_the_form` | One fetch of `/forms/form.json` returns vars, struct, model, conf, settings, and barcodes. Vars name country, region, and city. Conf is an array of screens and the first advice is `Личные данные`. The model `uniqueId` is `FJVFOMEICW`. The model has no `jv_rights` key. |
| `documents_second_country_ask_is_cached` | Two sequential `enums("ru", "country", {})` calls fetch `/dbenums/country_ru.json` once. Each call receives its own array. Splicing one leaves the cached list, which still contains id `219`, Россия. |
| `documents_concurrent_asks_share_one_request` | Two overlapping asks for the same key fetch once and both receive the list. |
| `documents_failed_refresh_keeps_the_list` | After the country list is cached, a failed refresh returns that list and an error. The cache still holds the list. |
| `documents_failed_refresh_shares_the_result` | A caller that joins a failed refresh receives the same result object: the cached list and the error. |
| `documents_enum_path_rejects_raw_text` | Locale `en` and integer ids build the city path. A locale or an id that is not an integer is rejected. |
| `documents_failed_counts_hides_the_query` | A failed counts or search error names the route and the status. The email and the phone are absent. |

### Done when

`npm test -- documents_` passes.

## Enum fields

A place field is named three times, and its default is stored three times. This is a proposal for a later fold. It is not part of step 2.

| place | structure | vars | question | model row |
|---|---|---|---|---|
| city | `enm: "av_p_vars.avp0"` and `e_val: -1` | `avp0: { name: "city", depends: ["country", "region"] }` | `dbenum: "city"`, `dbenum_def: 17849` | `enm: "av_p_vars.avp0"`, `e_val: -1` |
| region | `enm: "av_p_vars.avp1"` and `e_val: -1` | `avp1: { name: "region", depends: ["country"] }` | `dbenum: "region"`, `dbenum_defs: [3948]` | `enm: "av_p_vars.avp1"`, `e_val: -1` |
| country | `enm: "av_p_vars.avp2"` and `e_val: 219` | `avp2: { name: "country", depends: [] }` | `dbenum: "country"`, `dbenum_defs: [219]` | `enm: "av_p_vars.avp2"`, `e_val: 219` |

The stored field keeps one list name and the parent names. `av_p_vars.avpN` and the matching `avpN` object go away. `dbenum` goes away because it repeats that name.

```json
"city_db": { "list": "city", "parents": ["country", "region"] }
```

The default id is stored once, on the question (`default: 219` for country, `17849` for city, `3948` for region). The empty visitor stores `null` for region and city. Build writes the question default when the value is still `null`. The structure node and the model row do not each keep `e_val` and `enm`.

A questionnaire option list stays under its question id (`opt_1`). The structure key `opt: "av_p_vars.opt_1"` repeats that id and goes away with the same fold.

## Step 3. Leaves

[Back to summary](#summary)

The structure becomes leaves. Questions are not bound yet.

### Work

1. `leaves(struct, vars)` walks the visitor shape.
2. A parametric string leaf keeps `possible_links.str`. An enum leaf keeps `jvenum_field` and the var name with the `av_p_vars.` prefix removed. An option leaf keeps `jvopt_field` and the key `opt_N`.
3. A repeating leaf keeps `rel_prop_unique` and `_filter_fields_`.
4. A path that is not in the structure is an error. The message names the path.

### Test scenarios

| Scenario | Assert |
|---|---|
| `leaves_string` | A parametric string leaf reports the `str` link. |
| `leaves_enum_name` | An enum whose var is `av_p_vars.country` reports the name `country`. |
| `leaves_option` | An option field reports the key `opt_1`. |
| `leaves_phone_filter` | A repeating phone leaf filtered to `phone_type` 2 keeps that filter. |
| `leaves_missing_path` | Asking for a path the structure does not have returns an error that names the path. |

### Done when

`npm test -- leaves_` passes.

## Step 4. Values

[Back to summary](#summary)

`build` binds questions onto the copied model. `setValue` writes through the leaf.

### Work

1. `build(documents, locale)` copies the model, binds each body id and each `hiddenq` id to its leaf, and stores one question per id.
2. A body item that is an array is one row of question ids.
3. `setValue(state, id, locale, value)` writes a parametric string to `node[locale].str`, an enum to `e_val`, one option id or a list of ids, and a repeating value onto the primary link.

### Test scenarios

| Scenario | Assert |
|---|---|
| `values_same_id_is_one_question` | An id that appears on two screens is one entry in the question map. |
| `values_string_writes_the_locale` | Setting a string in `en` writes `node.en.str` and leaves `node.ru.str`. |
| `values_enum_empty_is_null` | The model file's region and city `e_val` of `-1` are `null` after build. Country `219` stays `219`. |
| `values_checkboxes_write_a_list` | A `chboxes` question without `single_select` stores a list of ids. |
| `values_single_select_writes_one_id` | The same type with `single_select` stores one id. |
| `values_repeating_writes_the_primary` | A repeating write updates the link whose `jvrel_prop.primary` is set. |
| `values_hidden_stays_in_the_map` | A `hiddenq` id is in the map with `hidden` set. |

### Done when

`npm test -- values_` passes.

## Step 5. Walk

[Back to summary](#summary)

Next and Back move the step. The check from the next step is not applied yet: a step with no required questions advances.

### Work

1. The first step is the first screen whose `for_locales` contains the locale, plus every later screen that shares its `screen_block`.
2. `next` walks the current screen's `next` rules in order. No `conditions` means the rule passes. `operator` `or` passes when one clause matches. Any other operator is `and`. A list value matches when it contains `value`. A scalar matches with strict equality.
3. A screen block uses the `next` rules of its last screen. The walk pushes the step. `back` pops one and does not run the rules.
4. A locale change replaces a step whose screens exclude the new locale with the nearest later step that includes it. Stored answers stay.
5. A rule step of `-1` is the end. A rule whose step the config marks as payment is the end.

### Test scenarios

| Scenario | Assert |
|---|---|
| `walk_unconditional_next` | A rule with no conditions moves to its step. |
| `walk_and_needs_every_clause` | An `and` rule with one failing clause is skipped. The following rule runs. |
| `walk_or_passes_on_one` | An `or` rule passes when one clause matches. |
| `walk_other_operator_is_and` | An operator other than `or` requires every clause. |
| `walk_list_contains_the_value` | A checkbox list matches a clause when the value is one of the ids. |
| `walk_screen_block_is_one_step` | Two screens that share `screen_block` are both on the first step. Next reads the second screen's rules. |
| `walk_back_pops` | Back returns to the previous step and leaves the answers in place. |
| `walk_locale_skips_and_keeps_answers` | Switching to `en` leaves a `ru`-only screen and keeps the value written on the previous step. |
| `walk_end_is_minus_one` | The matching rule's step `-1` sets the end flag. |
| `walk_payment_is_the_end` | A rule marked as payment sets the end flag. |

### Done when

`npm test -- walk_` passes.

## Step 6. Check

[Back to summary](#summary)

Next runs the step check first. Phone digits and email shape land with those controls.

### Work

1. `check(state, step, locale)` returns the errors on that step, in question order.
2. A required question with an empty value fails. `req_loc` applies only to that locale. A `req_index` group fails when every question in the index is empty.
3. `other_req` fails when "other" is selected and the text line is empty.
4. `next` does not change the step while the check returns an error. The first error is the one a layout will scroll to.

### Test scenarios

| Scenario | Assert |
|---|---|
| `check_required_empty` | An empty required question is the first error. Next stays on the step. |
| `check_required_for_one_locale` | `req_loc.ru` fails in `ru` and passes in `en` when the value is empty. |
| `check_req_index_group` | Two questions that share `req_index` pass when one of them has a value. |
| `check_other_required` | "Other" selected with an empty line is an error. A filled line passes. |
| `check_valid_step_advances` | A step whose questions pass moves to the rule's step. |

### Done when

`npm test -- check_` passes.

## Step 7. Place

[Back to summary](#summary)

Country, region, and city are fields of one address object.

### Work

1. Bind records the address path. `country_db` depends on nothing, region depends on that country, city depends on that country and region.
2. Countries load once per locale at build. Regions load when the country id is set. A `null` country leaves region and city unset and does not call `enums` for them. When the country is set, city files for that country start with the default region, and the other regions load through the same queue as the phone-mask files. At most 10 of those files are in flight. Each finished city file is added to the local city index. `searchCities` reads that index. A region already loaded is not requested again. Setting a city reads the index, and waits if that city's file has not arrived yet.
3. Changing country sets that address's region and city to the default row of the variable list just loaded (`d: true`, otherwise the question default when that id is in the list, otherwise the only row). It does not write `null`. Region is not a UI field. Setting a city writes that city and the region whose list contains it. A second address object is left as it was.
4. `dbenum_def` writes its id only when the value is `null` and the id is in the list just loaded.

### Test scenarios

| Scenario | Assert |
|---|---|
| `place_country_loads_at_build` | Build asks for `country` once. It does not ask for `region` or `city`. |
| `place_region_waits_for_country` | With country `null`, region is `null` and `enums` was not asked for `region`. Setting a country loads regions and stores the default region id. |
| `place_city_waits_for_both` | City loads only after country and region are both set. The request carries both ids. |
| `place_country_change_sets_defaults` | A new country sets region and city to the default ids from the loaded lists. The previous city id is gone. |
| `place_region_is_not_a_control` | Region is not a UI field. Writing it through `setValue` leaves the stored id. |
| `place_city_sets_the_region` | Setting a city writes that city and the region whose list contains it. Country stays. |
| `place_cities_load_ten_at_a_time` | The default region's city file is requested first. At most 10 city files are in flight, and a phone-mask file waits while those slots are full. A name is searchable only after its file arrives. A second load of the same country does not request those files again. |
| `place_second_address_stays` | Changing the first address's country leaves the second address's country, region, and city. |
| `place_default_only_when_null` | `dbenum_def` fills a `null` country. It does not replace a country the visitor already has. |
| `place_failed_list_keeps_the_previous` | A failed region refresh leaves the previous region list and sets the error on that control. |

### Done when

`npm test -- place_` passes.

## Step 8. Phones

[Back to summary](#summary)

`PhoneList` reads the country on its own address.

### Work

1. The control lists the repeating leaf's rows that match `_filter_fields_`. Add appends a link with `jvlink_id` `-1` and `primary` false. Remove deletes a row and refuses to delete the last one. Primary clears `primary` on the other rows of this list.
2. Selecting a calling code on the phone loads that country's calling code from `country_phone_code.json` and its masks from `country_phone_mask.json`. One country can have several masks, separated by `; `. The area code is in parentheses. Russia is `+7 (ddd) ddd-dd-dd`. The field keeps that country pattern until a stored city code is fully typed, and it chooses among the country's patterns by the leading digits of the stored codes. The stored value is `str_number`: calling code plus national digits. `country_code`, `city_or_operator`, and `number` are left untouched. The address country does not choose this code.
3. The masks used while typing are the per-region files `country-{countryId}/region-{regionId}/city_phone_mask.json`. A city row holds the mask, such as `+375 (1715) d-dd-dd` or `+7 (83147) d-dd-dd`. Selecting a calling code applies the country pattern before the region files finish. It requests the current region first and loads the other region files in the background through the same queue of 10 as the city files. Each file that arrives reapplies the mask. A region switch uses the loaded file, or fetches that one file if the background load has not started it. The field applies the stored mask whose code matches the typed digits. Until a per-region mask matches, the field uses the country mask. Deleting back out of the city code restores it. The field does not name the city or the operator. A city or operator image on the phone would come from the digits, and it does not follow the address region or city.
4. No calling code on the phone means an empty calling code and a digit field. Changing the address loads phone masks in the background while that phone has no calling code of its own and no number. An entered number stays, with its calling code. Selecting a calling code keeps the national digits and replaces the prefix, on this phone only. The form does not depend on libphonenumber. The JSON was filled from its metadata and geocoding files.
5. A value with no national digits is empty for the step check. The national part is what remains after the calling code loaded for that address. `unique_num` greater than zero calls `counts("phone", str_number)` when the step is checked.

### Test scenarios

| Scenario | Assert |
|---|---|
| `phones_mask_follows_this_country` | Selecting calling code Russia loads `+7 (ddd) ddd-dd-dd`. Selecting Kazakhstan loads the masks for `222`. |
| `phones_region_masks_load_for_the_country` | Selecting Russia returns with `+7 (ddd) ddd-dd-dd` and requests the current region first. At most 10 mask files are in flight. A typed `495` stays on that pattern until the Moscow file arrives, then becomes `+7 (495) ddd-dd-dd`. A second request for a region already started does not fetch it again. |
| `phones_mask_uses_the_region_file` | On the Minsk region, typing `1715` uses the stored Berezino mask `+375 (1715) d-dd-dd`. Typing `171` stays on the stored Minsk mask `+375 (17) ddd-dd-dd`. |
| `phones_no_country_is_digits` | With no calling code selected, the code is empty and the national part is stored as digits. An address country change leaves that number. |
| `phones_country_change_is_local` | An entered number stays when this address's country changes. The other address's number and mask stay. Selecting a calling code on this phone replaces the prefix and keeps the national digits. |
| `phones_address_country_loads_masks_in_background` | An empty phone follows the address country in the background. After a number is entered, a later address country leaves that number and its calling code. |
| `phones_stores_str_number` | The link's `str_number` is the calling code plus the digits. The split fields are absent. |
| `phones_no_digits_is_empty` | With Russia loaded, `+7` fails and `+7123` does not. With Belarus loaded, `+375` fails and `+3751` does not. With the US loaded, `+1` fails. |
| `phones_filter_splits_lists` | A mobile filter and a fax filter on the same leaf show different rows. |
| `phones_last_row_stays` | Remove on the only row leaves that row. |
| `phones_primary_is_one` | Marking a row primary clears `primary` on the others. |
| `phones_unique_num` | A count at `unique_num` sets the error. The counts call receives that `str_number`. |

### Done when

`npm test -- phones_` passes.

## Step 9. Emails

[Back to summary](#summary)

`EmailList` is the repeating `emails` leaf.

### Work

1. Add, remove, and primary work as they do for phones. One row is `{ email, jvrel_prop.primary }`.
2. The step check rejects a value that has no `@`, an empty local part, or no dot in the domain. `unique_num` calls `counts("email", value)`.
3. When the visitor came from search and the question is `dontchange`, the first edit returns a warning and does not write. The next edit writes. An email question uses the email sentence. Any other question uses the searched-person sentence. `disabled` never writes.
4. A `req_index` shared by the personal email and the company email passes when either one has a value.

### Test scenarios

| Scenario | Assert |
|---|---|
| `emails_needs_a_domain_dot` | `a@b` fails. `a@b.c` passes the shape check. |
| `emails_unique_num` | A count at `unique_num` sets the error and the counts call receives the address. |
| `emails_dontchange_warns_once` | The first edit of a searched visitor returns the email warning and keeps the old address. The second edit stores the new one. |
| `emails_dontchange_surname_has_its_own_warning` | The first edit of a searched visitor's surname returns the searched-person warning and keeps the old surname. The second edit stores the new one. |
| `emails_disabled_keeps_the_value` | A `disabled` question ignores `setValue`. |
| `emails_one_of_the_pair_is_enough` | The personal email filled and the company email empty passes the shared `req_index`. |

### Done when

`npm test -- emails_` passes.

## Step 10. Choices

[Back to summary](#summary)

`ChoiceList` renders a questionnaire whose var is an array, or whose leaf key is `opt_N`.

### Work

1. `select` stores one id. `chboxes` stores a list, or one id when `single_select` is set. `tree` stores the same, and each option keeps its `opts` depth.
2. `other` adds a text line. `binded_other` writes that text to the sibling question. An id that is not in the list is rejected unless `other` is set.
3. `dbenum_def` and `dbenum_defs` write once, at build, when the stored value is empty. Options sort by `priority`, then by the locale label. A default option has priority `0`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `choices_select_stores_one_id` | Choosing an option sets that id. |
| `choices_checkboxes_store_a_list` | Two checked options store both ids. |
| `choices_single_select_stores_one` | Checking a second option replaces the first. |
| `choices_tree_keeps_depth` | A child option reports the depth of its `opts` parent. |
| `choices_other_line` | Selecting "other" and typing stores the id and the line. |
| `choices_binded_other_writes_the_sibling` | The text lands on the sibling question's value. |
| `choices_unknown_id_is_rejected` | An id absent from the list is an error when `other` is unset. |
| `choices_default_once_at_build` | The default id is written when the value is empty. A later `setValue` is kept. |
| `choices_sort_by_priority` | A default option is first. The rest follow the locale label. |

### Done when

`npm test -- choices_` passes.

## Step 11. Step

[Back to summary](#summary)

`Step` draws the current step with the shared controls. The frame is one column of Tailwind utilities. Add `@tailwindcss/vite`, `tailwindcss`, and `app/index.css` here. `root.tsx` imports that file.

### Work

1. `Step` shows the advice, then each body item. A question picks `PlaceSelect`, `PhoneList`, `EmailList`, `ChoiceList`, or a text input from its leaf. The column is `flex flex-col`. A control is `w-full`. Its label is `block`.
2. An array body item is one row, `flex flex-col gap-4`. A hidden question is not drawn. The locale control calls the engine's locale change.
3. Next shows the first check error on its question. Back shows the previous step. The error text is `text-red-700`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `step_shows_the_advice` | The advice for the locale is the heading. |
| `step_row_is_one_group` | A body item that is an array renders its questions in one group. |
| `step_hidden_is_omitted` | A `hiddenq` question is absent from the document. |
| `step_place_phone_email_choice` | An address, a phone, an email, and a select each mount their control. |
| `step_error_sits_on_the_question` | An empty required field keeps the step and marks that question. |
| `step_locale_moves_the_screen` | The locale control switches the heading to the `en` advice and skips a `ru`-only screen. |

### Done when

`npm test -- step_` passes.

## Step 12. Layouts

[Back to summary](#summary)

The phone layout is the one step 1 served for `Sec-CH-UA-Mobile: ?1`. This step fills that frame in, and splits the kiosk layout into `ipad` and `web` from the viewport. The step component stays the one from step 11. Each frame is a set of Tailwind classes on the same elements.

`app/index.css` adds the tokens the default scale does not have:

```css
@import "tailwindcss";

@theme {
  --container-kiosk: 48rem;
  --container-web: 36rem;
  --spacing-bar: 4.5rem;
}
```

`max-w-kiosk`, `max-w-web`, and `pb-bar` are the utilities those tokens create.

### Work

1. On the kiosk layout, a touch device whose short side is at least 768 uses `ipad`. The column is `mx-auto flex w-full max-w-kiosk flex-col items-center`. A body row is `flex flex-row gap-4`. Back and Next are `fixed inset-x-0 bottom-0`. The step has `pb-bar` so the bar does not cover the last question.
2. On the kiosk layout, a pointer device whose width is at least 768 uses `web`. The column is `mx-auto w-full max-w-web`. A body row stays `flex flex-row gap-4`. Back and Next are `mt-6 flex`, under the questions.
3. The phone layout uses `flex w-full flex-col`. A body row is `flex flex-col`. Labels stay `block` above the controls. Back and Next are `fixed inset-x-0 bottom-0`, and the step has `pb-bar`.
4. A resize on a kiosk response switches the column classes between `max-w-kiosk` and `max-w-web` and keeps the step index and the visitor. It stays the kiosk layout.

### Test scenarios

| Scenario | Assert |
|---|---|
| `layouts_touch_768_is_ipad` | A kiosk response at a touch viewport of 768 has `max-w-kiosk` on the column, `flex-row` on the body row, and `fixed` on the bar. |
| `layouts_pointer_768_is_web` | A kiosk response at a pointer viewport of 1024 has `max-w-web` on the column and `mt-6` on the bar. The bar is not `fixed`. |
| `layouts_phone_header_stacks` | `Sec-CH-UA-Mobile: ?1` has `flex-col` on the body row and `fixed` plus `pb-bar` on the frame. |
| `layouts_resize_keeps_the_step` | Narrowing a kiosk window switches the column between `max-w-kiosk` and `max-w-web` and keeps the same step and the same answers. |

### Done when

`npm test -- layouts_` passes.

## Step 13. Register

[Back to summary](#summary)

The `/register` loader builds the form. The end of the walk can post.

### Work

1. The route loader calls `FormClient.load` and `build`. The page renders `Step` inside the layout from step 12.
2. Landing on step `-1` posts the visitor when settings have `print_on_save`. Without that flag `save` is not called. The badge line, the zone, and the return to the greeting are step 14.
3. Idle is two minutes from the last pointer or key, or the `reset` query in seconds. Idle restores the empty model. The screen that comes back is the greeting from step 14.
4. `no_ticket_code`, `no_printer`, and `invalid_phone_email_combo` from `save` are shown on the open step. Those results are not queued.

### Test scenarios

| Scenario | Assert |
|---|---|
| `register_loader_shows_the_first_step` | Opening `/register` shows the first screen's advice, `Личные данные`. |
| `register_print_on_save_posts` | Next that lands on `-1` calls `save` with the visitor. |
| `register_end_without_the_flag` | The same landing with `print_on_save` unset does not call `save`. |
| `register_idle_restores_the_model` | After the idle interval the typed value is gone. |
| `register_reset_query` | `?reset=1` uses a one-second idle. |
| `register_save_error_on_the_step` | A `save` result of `no_printer` shows that sentence on the open step. |

### Done when

`npm test -- register_` passes.

## Step 14. Kiosk finish

[Back to summary](#summary)

`/register` opens on a greeting, tells the visitor where the badge is, and returns there so the next person starts empty. A save that never reaches the local server stays in a queue and retries while that next person registers.

`form-app` (`reg-screens.component.ts`, `end-screen.component.ts`) opens on `start-screen`. A successful local save writes the badge line, then after 15 seconds clears the model and resets navigation to that greeting. A `ticketStatus` other than 1 writes the pay line instead. `POST /datapost` returns `zoneName.ru`, the printer sector. The screen stores that response in `zones` and does not show it. A failed save retries on the same screen, up to 5 times, with the mask up, then alerts and stops. Idle is two minutes without a pointer or key.

### Work

1. `/register` opens on the greeting, inside the layout from step 12. The greeting has the locale control and a start button. Start shows the first step. The visitor is the empty model. `register_loader_shows_the_first_step` finds `Личные данные` after Start.
2. After the walk ends, the screen shows the badge line for 15 seconds, then the greeting, with a new empty model. The line is `Получите Ваш бейдж на стойке выдачи бейджей.` / `Take your badge at the registration desk.`. A `ticketStatus` other than 1 shows `Вы зарегистрированы. Пожалуйста оплатите участие на кассе.` / `You are registered. Please pay for your participation at the ticket desk.` A non-empty `zone_name` from `save` is the line instead of the badge line. The chrome strings live in `app/locales/{ru,en}.json`.
3. A save that fails before a response, or with a 5xx, appends the visitor to `localStorage` under `registration-form.outbox` and returns to the greeting. The badge line stays off. The queue sends one saved visitor at a time, on load and every 5 seconds. A success removes that visitor. The next registration can be filled and queued while an earlier one is still retrying. This replaces the 5 blocking retries.

### Test scenarios

| Scenario | Assert |
|---|---|
| `finish_greeting_starts_the_form` | Opening `/register` shows the start button. The first step's advice appears after Start. |
| `finish_desk_line_then_the_greeting` | A save with an empty `zone_name` and `ticketStatus` 1 shows the desk line. After 15 seconds the greeting is back and the typed value is gone. |
| `finish_zone_replaces_the_desk_line` | A save that returns `zone_name` `Сектор А` shows that name. The desk line is absent. |
| `finish_failed_save_is_queued` | A save that rejects before a response stores the visitor, returns to the greeting, and leaves the desk line off. The next successful retry removes it from the queue. |
| `finish_queue_does_not_block` | While the first save is still failing, a second visitor can be started, finished, and stored behind it. |

### Done when

`npm test -- finish_` passes.

## Step 15. Desk

[Back to summary](#summary)

`/desk` is the operator frame around the same `Step`.

### Work

1. The frame adds a sticky header with category, a paid/unpaid pair, printer, and packets. Every reachable screen is on one page. Each passing `next` rule opens its whole path, and a failed branch stays hidden, including the screens that follow only that branch. Scalar fields are a third of the row from 865px, and checkbox and radio lists use the same three columns. The page is `flex flex-col`. The operator fields are the first block. On a narrow window the main column uses the phone classes from step 12 (`flex w-full flex-col`, `pb-bar`) and those fields stay above the questions.
2. Category and ticket status write `category` and `subscribtion.jvrel_prop.ticket_status`. Packets write `ext_packets`. Save is fixed to the bottom of the screen and posts the visitor and the printer name.
3. The desk does not idle-reset. It has no greeting and no queue. A failed save stays on the page.

### Test scenarios

| Scenario | Assert |
|---|---|
| `desk_omits_search` | Search and new visitor are absent. The personal block and the company block are both on the page. |
| `desk_condition_shows_the_block` | A next rule that requires a surname hides the following block until that surname is typed. |
| `desk_condition_tree_shows_one_path` | Surname Иванов shows B, D, E, F. Surname Петров shows C, H, F. Any other surname shows neither path. |
| `desk_fields_fit_three_across` | At width 1024 a scalar field uses the three-column class, the operator bar does not wrap, and Next is absent. |
| `desk_save_posts_printer` | Save calls `save` with the visitor, the category, the ticket status, and the selected printer. |
| `desk_saves_with_required_fields_empty` | Operator Save posts while required questions are empty. Those labels have no required mark. |
| `desk_narrow_keeps_the_fields` | Width 767 still shows category, the unpaid tab, and the printer on one line above the questions. |
| `desk_does_not_idle` | Past the kiosk idle interval, the typed answer is still there. |
| `desk_failed_save_stays` | A failed save leaves the typed value on the page and leaves the queue empty. |

### Done when

`npm test -- desk_` passes.
