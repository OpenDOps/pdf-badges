# Printing — step 8

Build sequence for [porting.md](porting.md) step 8. The queues, the printer record, the routing, and the badge bytes are [printing.md](printing.md). What is decoded when the event opens, and which layer is drawn before the visitor, is [printing-preload.md](printing-preload.md). How the worker shares the core is [design.md](design.md).

This plan adds the `ticket-render` crate and the CUPS client on the server thread. `ticket-render` is the Scala and C badge: `badge.cfg` becomes a layout, then PNG bytes or a 1-bit graphic. `struct_to_pdf` is the other renderer: a YAML page becomes PDF, PNG, BMP, or ZPL. The worker uses the YAML page when that category has one, and `ticket-render` otherwise. Both are decoded into memory when the event opens and again when sync stores one category. A print draws from that memory. The server finds queues, picks one, wraps ZPL when the queue is Zebra or TSC, and submits one IPP job. It does not download CUPS filters or rewrite a PPD. Sync stores the badge files and does not draw.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 8 is done when steps 1 through 18 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Layout](#step-1-layout) | The badge document the crate draws | done |
| [2. Replace](#step-2-replace) | `{{field}}` and the area condition | done |
| [3. Config](#step-3-config) | `badge.cfg` becomes that document once | done |
| [4. Page](#step-4-page) | Background and text become a PNG | done |
| [5. Codes](#step-5-codes) | EAN-13, Code 128, QR, and a photo | done |
| [6. Graphic](#step-6-graphic) | The 1-bit payload, with no ZPL wrapper | done |
| [7. Look](#step-7-look) | A PNG on disk, opened by hand, before any queue exists | done |
| [8. Page file](#step-8-page-file) | A YAML page becomes PDF bytes | not started |
| [9. Raster file](#step-9-raster-file) | That page becomes PNG and BMP | not started |
| [10. ZPL file](#step-10-zpl-file) | That page becomes ZPL | not started |
| [11. Prepare](#step-11-prepare) | Both badges are decoded on a switch and when sync stores one category | not started |
| [12. Queues](#step-12-queues) | `CUPS-Get-Printers` becomes one `Printer` per queue | not started |
| [13. Devices](#step-13-devices) | A live device marks the queue connected. A new device is installed once | not started |
| [14. Files](#step-14-files) | Settings on the device, routing on the event | not started |
| [15. Pick](#step-15-pick) | Connected, used, category, internet, zone, oldest `last_print` | not started |
| [16. Submit](#step-16-submit) | One `Print-Job`: ZPL raw, or a PNG, options on the job | not started |
| [17. Retry](#step-17-retry) | An aborted job is cancelled and sent again. A stopped queue is resumed | not started |
| [18. Worker](#step-18-worker) | `POST /print` draws from the prepared badge, yields, and submits | not started |

Shared rules:

- `ticket-render` is the package `modules/ticket-render`. The root `Cargo.toml` depends on it by path. The crate does not depend on `rust-reg`, `lopdf`, or CUPS. Its tests are `cargo test -p ticket-render`. They do not open a socket.
- CUPS lives in `registration_server::cups` in `src/registration_server/cups.rs`. Tests are `cargo test --lib registration_server::cups`. The stand-in is HTTP on `127.0.0.1` and speaks IPP. Tests do not start cupsd and do not open a device.
- A test that writes `base/printer_conf` or `base/{event_id}/pr_{name}.json` uses a temp directory.
- `ticket-render` does not call `struct_to_pdf::template::fill`. A YAML badge uses `struct_to_pdf` for its own fill and draw. The two renderers do not share a layout file.
- The print worker stays one job on the server thread, yielding between pages, with the gate closed while the job runs. `PRINT_QUEUE_DEPTH` stays 16.
- Sync stores the badge files and does not draw. Step 11 rebuilds that category in memory after the bytes are on disk, and rebuilds every category when the event opens. This plan does not send `getbadge`.
- Step 7 of the sync plan stays green: `cargo test --lib registration_server::sync::tests::queue`. The gate tests stay green: `cargo test --lib registration_server::gate`.

## Step 1. Layout

[Back to summary](#summary)

The crate's document is the layout JSON. Render does not read it yet.

### Work

1. Add `modules/ticket-render` and the path dependency.
2. `Layout` is the page width, the page height, the dots per point, and the areas. An area is a box, alignment, rotation, color, text, condition, and a type: text, barcode, or photo. A text area has the font size, families, weight, style, and text parts. A barcode area has the barcode type. A photo area has no font.
3. `load` reads that JSON. `store` writes it. A second `load` returns the same layout.

### Test scenarios

| Scenario | Assert |
|---|---|
| `layout_roundtrip` | A layout with one text area, one barcode area, and one photo area is stored and loaded. Width, height, dots per point, box, condition, barcode type, and the text part's font match. |
| `unknown_area_type_is_rejected` | A type other than text, barcode, or photo is an error. The message names the type. |

### Done when

`cargo test -p ticket-render -- layout` passes.

## Step 2. Replace

[Back to summary](#summary)

`{{field}}` is replaced before anything is drawn. The condition decides whether the area is drawn.

### Work

1. `replace(text, values)` scans `{{` … `}}` once. The name is trimmed and lowercased. The value is inserted as given. A missing name inserts nothing. `&`, `<`, and `>` are copied through.
2. An area with an empty condition is kept. Otherwise the condition is a key in the same map. The area is dropped when the value is missing, empty, `false`, or `0`.
3. This step returns the kept areas and their replaced strings. It does not decode PNG.

### Test scenarios

| Scenario | Assert |
|---|---|
| `field_is_lowercased` | `{{Name}}` with `name` = `Ann` becomes `Ann`. |
| `missing_field_is_blank` | `Hello {{name}}` with an empty map becomes `Hello `. |
| `ampersand_is_kept` | `{{company}}` with value `A&B` becomes `A&B`. |
| `condition_hides_the_area` | Condition `possiblecat` drops the area when the value is missing, empty, `false`, or `0`. An empty condition keeps the area. Value `1` keeps it. |

### Done when

`cargo test -p ticket-render -- replace` passes, and step 1 stays green.

## Step 3. Config

[Back to summary](#summary)

`badge.cfg` is the Java object stream sync stores. The crate reads it once and writes the layout JSON beside it.

### Work

1. `decode_cfg` reads `serialVersionUID` 5, the page size, the dots per point, and each area in the order `OutputAreasHolder.writeObject` and `OutputArea.writeObject` write them. Another version is an error.
2. The text-part font-family count is the count the writer emits. A captured fixture decides that count. The decoder is not adjusted until a fixture fails.
3. `store_layout(dir, cfg_bytes)` writes `layout.json` in that directory. A second call with the same bytes leaves the same JSON.

### Test scenarios

| Scenario | Assert |
|---|---|
| `cfg_becomes_layout_json` | The committed fixture cfg decodes to the width, height, area count, and first area text recorded next to the fixture. `layout.json` loads with step 1. |
| `other_version_is_rejected` | A stream whose version is not 5 is an error. No `layout.json` is written. |

### Done when

`cargo test -p ticket-render -- cfg` passes, and steps 1 and 2 stay green.

## Step 4. Page

[Back to summary](#summary)

A layout and a value map become PNG bytes. The background is `bg.png` in the layout directory.

### Work

1. `render_png(layout, values, dpi, bg)` returns PNG bytes. The pixel size is the layout's inch size times `dpi`, after the same dots-per-point scale the Java holder uses.
2. The background is drawn first, scaled to that pixel size. Each kept text area is drawn in its box, with its alignment and rotation. The face is the file named in [Fonts](printing.md#fonts). A family with no file uses DejaVu Sans 400, compiled into the crate.
3. The crate does not write a file.

### Test scenarios

| Scenario | Assert |
|---|---|
| `png_has_the_page_size` | A 2 by 1 inch layout at 100 dpi is 200 by 100 pixels. |
| `text_is_drawn` | `{{name}}` with `name` = `Ann` changes a pixel inside the text box from the background color. |
| `hidden_area_is_not_drawn` | The same layout with the condition value `false` leaves that pixel on the background color. |
| `otf_weight_is_selected` | Family `Sample` at weight `bold` is wider than weight `normal`. A missing italic uses that roman face. Both files are `.otf`. |

### Done when

`cargo test -p ticket-render -- page` passes, and steps 1 through 3 stay green.

## Step 5. Codes

[Back to summary](#summary)

Barcode areas, QR, and a photo are drawn on the same page.

### Work

1. A barcode area of type EAN-13 or Code 128 draws that symbol from the replaced text. Any other barcode type is an error.
2. `render_qr(text, size_px)` returns a PNG of that size. It is the same encoder the page uses when a barcode type is QR.
3. A photo area draws the bytes the caller passes, in the area box. A missing photo leaves the background.

### Test scenarios

| Scenario | Assert |
|---|---|
| `ean13_modules` | Code `9891160081678` produces the guard and digit modules of EAN-13. The quiet zones are present. |
| `code128_modules` | The string `REG` produces the Code 128 start, data, checksum, and stop modules. |
| `qr_roundtrip` | `render_qr("https://kuprin.su/", 200)` is a 200 pixel PNG. Decoding it returns that string. |
| `photo_is_placed` | A solid red image passed for the photo area sets a pixel inside the photo box to red. An absent photo leaves the background. |

### Done when

`cargo test -p ticket-render -- codes` passes, and steps 1 through 4 stay green.

## Step 6. Graphic

[Back to summary](#summary)

The ZPL queue needs the 1-bit graphic. The crate returns that payload. The `^XA` wrapper is step 16.

### Work

1. `render_graphic(layout, values, dpi, bg)` returns the `~DG` payload: the byte length line and the hex rows. Pixels are thresholded the way the C writer thresholds them.
2. The bytes contain no `^XA`, `^FO`, `^XZ`, or printer setting.

### Test scenarios

| Scenario | Assert |
|---|---|
| `graphic_is_the_payload` | An 8 by 1 pixel black row is one hex line of `F`, with the `~DG` length header. The bytes do not contain `^XA`. |
| `graphic_matches_the_png` | The same layout rendered as PNG and as a graphic agrees on which pixels are black. |

### Done when

`cargo test -p ticket-render -- graphic` passes, and steps 1 through 5 stay green.

## Step 7. Look

[Back to summary](#summary)

The PNG from step 4 is written where the Scala client wrote a badge when no printer was connected, so it can be opened before any queue exists.

`PrintHelper.printBadge` catches every failure of the print, including `PrinterFactory.getAnyPrinter` throwing `RuntimeException("no printers found")` when the printer map is empty. The catch does not send a job. It creates `{base}/{event id}/genbadges` when that directory is missing and calls `renderBadge` with a `DefaultPrinter` that is not connected, at 720 dpi, format `png`, and `isPrintCert` false. `renderBadge` then lowers that dpi until the long side is at most 2400 pixels (1200 on the tiny build). The path passed in is `genbadges/{category}_{name}_{surname}_{patronymic}_{company}`, joined with `_`, empty fields left empty. The C writer appends `.png` when the format is `png` and a full path was given. The folder name in `PrintHelper.scala` is `genbadges`. When transliteration is on, the catch writes a second file with `-tr` after transliterating `company`, `cups_company`, `position`, `cups_position`, `city`, `cups_city`, `name`, `cups_name`, `surname`, `cups_surname`, `patronymic`, and `cups_patronymic`. Otherwise, when a double print is on, it writes a second file with `-d`. The call returns no queue.

### Work

1. `write_badge_png(dir, layout, values, bg)` writes `{dir}/genbadges/{category}_{name}_{surname}_{patronymic}_{company}.png`. It creates `genbadges` when missing. DPI starts at 720 and is lowered by the same long-side rule. The bytes are `render_png` from step 4.
2. The committed fixture visitor supplies category, name, surname, patronymic, and company. The test writes under `target/ticket-render/genbadges/` and leaves the file there.
3. This step does not send a job and does not wrap ZPL. The second `-tr` or `-d` file is step 18.

### Test scenarios

| Scenario | Assert |
|---|---|
| `badge_png_is_written` | The file path ends with `genbadges/` and the five fields joined by `_`, plus `.png`. The bytes are a PNG of the page size from step 4 at the lowered dpi. |

### Done when

`cargo test -p ticket-render -- look` passes, and steps 1 through 6 stay green. The PNG under `target/ticket-render/genbadges/` has been opened, and the fixture visitor's name is readable on the badge.

## Step 8. Page file

[Back to summary](#summary)

A category can draw from a `struct_to_pdf` page instead of the `ticket-render` layout. This step returns PDF bytes. PNG, BMP, and ZPL are the next two steps.

### Work

1. `badge/{category}/page.yaml` (`.yml` and `.json` are the same page) is loaded with `struct_to_pdf`. One visitor map is one row. Filling and drawing stay in that crate. `ticket-render` is not called.
2. The page draws `ifSt`, a barcode, and a photo, as [document-schema.md](../pdf-tooling/document-schema.md) specifies. An absent or empty `ifSt` draws the entry. Any other `ifSt` is a key in the visitor map, and the entry is skipped when that value is missing, empty, `false`, or `0`. A barcode `type` of `ean13` or `code128` draws that symbol from the filled payload. `qr` draws a QR code of that payload. A photo draws the bytes for `photo.field` in its rectangle. A missing photo leaves the rectangle empty.
3. The function returns PDF bytes. It does not write a file and does not send `Print-Job`.
4. A category with no page file returns no PDF. The old layout remains the other renderer. This step does not fall through to it.

### Test scenarios

| Scenario | Assert |
|---|---|
| `yaml_page_is_pdf` | A page whose template contains `{{name}}`, filled with `name` = `Ann`, is a PDF that contains `Ann`. |
| `if_st_hides_the_entry` | `ifSt` `possiblecat` with value `false` leaves that entry out of the PDF. Value `1` draws it. An empty `ifSt` draws it. |
| `yaml_barcode_is_drawn` | Type `ean13` with payload `9891160081678` draws that symbol in its box, quiet zones included. Type `qr` with payload `https://kuprin.su/` draws a QR code that decodes to that string. |
| `yaml_photo_is_placed` | Bytes for `photo.field` paint that rectangle. A missing photo leaves the rectangle empty. `ifSt` `false` on the photo draws nothing there. |
| `missing_page_file_is_not_pdf` | A category directory with only `layout.json` returns no PDF. The error names the missing page file. |

### Done when

`cargo test --lib registration_server::print::tests::page_file` passes, and steps 1 through 7 stay green.

## Step 9. Raster file

[Back to summary](#summary)

The same page becomes a PNG and a BMP. The PDF from step 8 is not the input. The page is drawn again at a requested dpi.

### Work

1. `render` of that page at a dpi returns PNG bytes and BMP bytes. The pixel size is the page's inch size times that dpi. The barcode, the QR code, and the photo are drawn into that raster. An entry whose `ifSt` hides it is left on the background.
2. The test writes both files under `target/registration-server/genbadges/` and leaves them there. They are not submitted.

### Test scenarios

| Scenario | Assert |
|---|---|
| `yaml_page_is_png` | A 2 by 1 inch page at 100 dpi is a 200 by 100 PNG. A drawn barcode changes a pixel inside its box. `ifSt` `false` leaves that pixel on the background. |
| `yaml_page_is_bmp` | The same page is a BMP of those dimensions. The photo rectangle is the supplied image. A missing photo leaves the background. |

### Done when

`cargo test --lib registration_server::print::tests::raster_file` passes, and step 8 stays green. The PNG under `target/registration-server/genbadges/` has been opened, and the fixture name is readable.

## Step 10. ZPL file

[Back to summary](#summary)

The same page becomes ZPL. The registration server wraps the graphic. `struct_to_pdf` does not speak ZPL.

### Work

1. The PNG from step 9 is turned into the `~DG` payload from step 6. A function on `ticket-render` takes those pixels and returns that payload. It does not read a layout. A barcode or a photo that step 9 drew is in those pixels. An entry step 9 hid is not.
2. The registration server wraps that payload as `^XA`, the seven Zebra settings (`^PR`, `^MD`, `^MM`, `^LS`, `^LT`, `~TA`), `^FO`, and `^XZ`. Step 16 sends those settings on a ZPL job.
3. The bytes are not sent to CUPS in this step.

### Test scenarios

| Scenario | Assert |
|---|---|
| `yaml_page_is_zpl` | The body starts with `^XA`, contains the `~DG` payload, and ends with `^XZ`. The stand-in sees no `Print-Job`. |

### Done when

`cargo test --lib registration_server::print::tests::zpl_file` passes, and steps 6 and 9 stay green.

## Step 11. Prepare

[Back to summary](#summary)

A print draws from memory. `badge.cfg` and the YAML page are decoded when the event opens, and again when sync stores a new file for one category. What that memory holds is [printing-preload.md](printing-preload.md).

### Work

1. `registration_server::print` keeps one map for the open event. The key is the category directory: the badge category, or that category plus 1000 for a certificate. A directory that has `page.yaml` (`.yml` and `.json` are the same page) stores the `Page`, the `TemplateIndex`, the font bytes, and the decoded images. A directory that has `badge.cfg` stores the layout from step 3, writes `layout.json` beside the cfg, and stores the decoded `bg.png` and the font bytes the layout names. A directory with both stores both. The page entry is what a badge print reads when it is present. The layout entry is what a certificate print reads.
2. `switch_to` and the startup `open_bound` build that map from every `badge/` directory of the event that is open after the call. The previous event's map is dropped only after the next map is complete. A switch to the id already open leaves the map unchanged. `clear` drops the map.
3. The sync store of `badge.cfg` and `bg.png` for one category of the open event replaces that category's layout entry. The sync store of a page file for one category of the open event replaces that category's page entry. The other categories stay. A store whose event id is not the open event leaves the map unchanged. This step does not send `getbadge`. The store is what the sync plan calls after the bytes are on disk.
4. A category whose cfg fails to decode, or whose page fails to parse, is kept as that error and is left out of the drawable entries. The other categories are still in the map. The switch still returns the new event id.
5. `render_png`, `render_graphic`, and the YAML renders receive these prepared values. That call does not open `badge.cfg`, `layout.json`, a page file, a font file, or an image file.

### Test scenarios

| Scenario | Assert |
|---|---|
| `switch_prepares_cfg_and_page` | `AAA` has `badge/7/badge.cfg` with `bg.png`, and `badge/3/page.yaml`. After `switch_to("AAA")` the map holds category 7's layout size and category 3's template field. Removing those files leaves both values in the map. |
| `switch_drops_the_previous_event` | `switch_to("BBB")` leaves `BBB`'s categories in the map. `AAA` has no entry. |
| `the_same_event_keeps_the_map` | After the files on disk change, `switch_to` of the open id still returns the first layout and the first page. |
| `a_cfg_update_reloads_one_category` | Storing a new `badge.cfg` for category 7 replaces that layout. Category 3's page is the one from the switch. |
| `a_page_update_reloads_one_category` | Storing a new `page.yaml` for category 3 replaces that page. Category 7's layout stays. |
| `a_broken_file_omits_that_category` | Category 7's cfg version is not 5. Category 3's page is prepared. `switch_to` returns the new event id. A lookup of category 7 is the version error. |
| `another_event_is_not_applied` | A store recorded for `BBB` while `AAA` is open leaves `AAA`'s map unchanged. |

### Done when

`cargo test --lib registration_server::print::tests::prepare` passes, and steps 1 through 10 stay green.

## Step 12. Queues

[Back to summary](#summary)

Search asks cupsd for queues and builds one `Printer` each. Devices and files come in the next steps.

### Work

1. `Cups::search` sends `CUPS-Get-Printers` and reads the queue name, device URI, make and model, state, and whether the queue accepts jobs.
2. Make `zebra technologies` or `tsc` sets `kind` to `zpl`. Any other make sets `raster`. Model `ztc gx430t` and make `evolis` set `dpi` 300. Other ZPL queues set `dpi` 203 and `width_in` 4.1. Evolis sets `width_in` 2.125. A raster queue leaves `width_in` empty.
3. `choices` are the PPD options except `PageSize`. `connected` is false until step 13. `installed` is true because the queue exists.

### Test scenarios

| Scenario | Assert |
|---|---|
| `queues_become_printers` | The stand-in lists a Zebra `ztc gx430t`, a TSC, an Evolis, and one other queue. The four records have kinds `zpl`, `zpl`, `raster`, `raster`, dpi 300, 203, 300, 300, and the ZPL and Evolis widths above. `PageSize` is not in `choices`. |
| `search_sends_get_printers` | The stand-in sees one `CUPS-Get-Printers` and no `Print-Job`. |

### Done when

`cargo test --lib registration_server::cups::tests::queues` passes, and steps 1 through 11 stay green.

## Step 13. Devices

[Back to summary](#summary)

A second IPP call marks queues connected. A device with no queue is installed once.

### Work

1. Search also sends `CUPS-Get-Devices`. A queue whose device URI is in that list has `connected` true. A queue whose URI is absent stays in the list with `connected` false.
2. A device with no queue is installed with one `CUPS-Add-Modify-Printer`. The PPD name is one `CUPS-Get-PPDs` returned for that make and model. Zebra uses the PPD name `zebra.ppd` when the stand-in lists it. Evolis uses `evolis-zeniusE.ppd` when it lists it. The queue name is make, model, and the USB serial: spaces become `_`, and the serial is cut at `&`.
3. A make with no PPD stays in the list with `installed` false. Search does not send `Add-Modify` for it.
4. A second search sees the queue it just created and does not send `Add-Modify` again.
5. A stopped queue, or one that is not accepting jobs, is resumed when it is first adopted. Saved option values are not sent.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_live_device_is_connected` | One queue URI is in the device list and one is not. The first printer is connected. The second stays, with `connected` false. |
| `a_new_device_is_installed_once` | A USB device `usb://Zebra Technologies/ZTC?serial=ABC&extra` and a listed `zebra.ppd` produce one `Add-Modify` whose queue name is `Zebra_Technologies_ZTC_ABC`. The second search sends no `Add-Modify`. |
| `no_ppd_is_not_installed` | A device whose make is absent from `CUPS-Get-PPDs` is `installed` false. The stand-in sees no `Add-Modify`. |

### Done when

`cargo test --lib registration_server::cups::tests::devices` passes, and step 12 stays green.

## Step 14. Files

[Back to summary](#summary)

Settings belong to the device. Routing belongs to the open event. Search loads both after the IPP lists return, and replaces the in-memory map in one assignment.

### Work

1. `base/printer_conf/{name}.json` is `values`. A missing file is written from the PPD defaults on that search. ZPL defaults are the Scala defaults: tracking `Web`, media `Direct`, mode `Cutter`, speed `10`, darkness `0`, shift `0`, top `0`, tear-off `0`. Evolis default is `InkType` `Black`.
2. `base/{event_id}/pr_{name}.json` is the routing record: sector name, used, zone, category, internet, second name, print certificate, auto print, no postscript. A missing file loads as used, with an empty sector name and the flags false. `last_print` is not in the file. It starts at 0.
3. `save_routing` writes every field, including when second name is empty.
4. Search does not send the file values to cupsd.

### Test scenarios

| Scenario | Assert |
|---|---|
| `missing_settings_are_written` | A Zebra queue with no settings file gets `base/printer_conf/{name}.json`. Darkness is `0` and speed is `10`. The stand-in sees no `Print-Job` and no option-setting operation. |
| `routing_roundtrip_without_a_second_name` | A record with empty second name, `print_cert` true, and `auto_print` true is saved and loaded with those three fields intact. |
| `missing_routing_is_used` | A queue with no `pr_{name}.json` loads used, with no zone and no category. |

### Done when

`cargo test --lib registration_server::cups::tests::files` passes, and steps 12 and 13 stay green.

## Step 15. Pick

[Back to summary](#summary)

A print names a visitor category, an internet flag, an optional zone, and an optional queue name. Pick returns one connected, installed queue, or an error.

### Work

1. An explicit name returns that queue when it is connected and installed.
2. Otherwise the candidates are connected, installed, and used. A queue with a category stays only when it equals the visitor category. A queue with no category stays only when no used queue has that category. The internet flag uses the same rule.
3. A requested zone drops a queue bound to a different zone. A queue bound to the requested zone beats a queue with no zone. The oldest `last_print` wins a tie. With no zone, the oldest `last_print` wins.
4. An empty candidate list is an error. A queue with `used` false is not a fallback. `auto_print` and `no_postscript` are not read.
5. A successful pick sets `last_print` on that queue.

### Test scenarios

| Scenario | Assert |
|---|---|
| `explicit_name_wins` | The named queue is returned even when another queue is older. |
| `category_and_internet_filter` | Category 7 drops a queue bound to category 3. A queue with no category is dropped when another used queue has category 7. The same holds for the internet flag. |
| `zone_prefers_a_bound_queue` | Zone 2 returns the queue bound to zone 2, not an older queue bound to zone 1, and not an older queue with no zone. |
| `oldest_last_print_wins` | Two unbound used queues: the one with the smaller `last_print` is returned, and its `last_print` changes. |
| `nothing_matching_is_an_error` | Every queue is disconnected, or every matching queue has `used` false. The error names no queue. `last_print` is unchanged. |

### Done when

`cargo test --lib registration_server::cups::tests::pick` passes, and steps 12 through 14 stay green.

## Step 16. Submit

[Back to summary](#summary)

One job is one `Print-Job`. The body is ZPL or a PNG. Options travel on the request.

### Work

1. A `zpl` queue sends the graphic from step 6 wrapped as `^XA`, the seven Zebra settings (`^PR`, `^MD`, `^MM`, `^LS`, `^LT`, `~TA`), `^FO` centered with `width_in`, and `^XZ`. The document format is raw. `zeMediaTracking`, `MediaType`, and `zePrintMode` are also IPP options.
2. A `raster` queue sends the PNG from step 4. The document format is `image/png`. Evolis adds `InkType`. The media size is the layout's inch width and height. `no_postscript` does not change the format.
3. The stand-in answers `job-state` completed. The call returns the job id.

### Test scenarios

| Scenario | Assert |
|---|---|
| `zpl_job_is_raw` | The body starts with `^XA`, contains `^PR10` and `^MD0` for the default settings, contains the graphic, and ends with `^XZ`. The document format is raw. The IPP options include `zeMediaTracking=Web`. |
| `raster_job_is_png` | An Evolis queue with `no_postscript` true sends `image/png`, `InkType=Black`, and a media size taken from the layout. The body is a PNG. The stand-in sees no request that writes a PPD. |

### Done when

`cargo test --lib registration_server::cups::tests::submit` passes, and steps 12 through 15 stay green.

## Step 17. Retry

[Back to summary](#summary)

Completion is `Get-Job-Attributes`. A failed job is cancelled and sent again, up to ten times.

### Work

1. After `Print-Job`, the client reads `job-state` and `job-state-reasons`. `completed` returns. `aborted`, or a state that cannot accept data, sends `Cancel-Job` and submits again.
2. The tenth failure returns an error. The stand-in has seen ten `Print-Job` requests and the cancels for the ones that aborted.
3. A queue that is stopped is resumed, then the job is sent. A queue that fails because it is stopped is resumed again before the next try.

### Test scenarios

| Scenario | Assert |
|---|---|
| `an_aborted_job_is_retried` | The first job is `aborted`. The stand-in sees `Cancel-Job`, then a second `Print-Job` that completes. The call returns the second job id. |
| `ten_failures_stop` | Every job aborts. The call returns an error. The stand-in has seen ten `Print-Job` requests. |
| `a_stopped_queue_is_resumed` | The queue state is stopped. The stand-in sees a resume, then one completed `Print-Job`. |

### Done when

`cargo test --lib registration_server::cups::tests::retry` passes, and step 16 stays green.

## Step 18. Worker

[Back to summary](#summary)

`POST /print` names a registration. The worker on the server thread picks the queue, draws with one renderer, and submits. The channel still holds one running job.

### Work

1. The request body is the registration id, the category, the internet flag, an optional zone, and an optional queue name. The worker loads the visitor fields from the open event.
2. A category whose prepared page from step 11 is present draws with steps 8 through 10. Otherwise it draws the prepared layout from step 11, or the prepared certificate layout at category plus 1000 when the queue has `print_cert`. A missing certificate layout uses category 999, then `-1`, then the smallest non-negative prepared category. The print reads that memory. It does not read `badge.cfg`, `layout.json`, a page file, a font file, or an image file. The dpi is the queue dpi, lowered so the long side is at most 2400 pixels. A caller may pass a lower cap. The test passes 1200 and sees that cap.
3. It picks with step 15. One page is one render and one submit. The worker then awaits `yield_now` before a second page. A transliterated copy and a double print are that second page. The second submit goes to the same queue. The worker does not sleep.
4. The transliterated keys are `company`, `cups_company`, `position`, `cups_position`, `city`, `cups_city`, `name`, `cups_name`, `surname`, `cups_surname`, `patronymic`, `cups_patronymic`.
5. When no connected printer is picked, the worker writes the PNG from step 7's path, `{event}/genbadges/{category}_{name}_{surname}_{patronymic}_{company}.png`, and does not send `Print-Job`. `isPrintCert` is false for that file. Transliteration writes the `-tr` file. A double print without transliteration writes the `-d` file. The response names that path and has no job id.
6. A submit error after a queue was picked is the HTTP error. That failure does not write `genbadges/`.
7. `POST /print/preview` returns the PNG from the renderer selected in item 2 and does not send `Print-Job`.
8. The gate is held from the start of the job until the last submit returns. A no-printer write holds the gate until the file is written.

### Test scenarios

| Scenario | Assert |
|---|---|
| `print_submits_one_job` | `POST /print` for a raster queue returns the job id. The stand-in has seen one `Print-Job` with `image/png`. |
| `no_printer_writes_genbadges` | No connected queue writes `{event}/genbadges/{category}_{name}_{surname}_{patronymic}_{company}.png` and sends no `Print-Job`. The response has no job id. |
| `a_second_page_yields` | A double print sends two jobs to the same queue. The gate is closed across both submits and open after the second returns. |
| `preview_does_not_submit` | `POST /print/preview` returns PNG bytes. The stand-in has seen no `Print-Job`. A category with a prepared page returns the PNG from step 9. A category with only a prepared layout returns the PNG from `ticket-render`. |
| `certificate_layout` | A queue with `print_cert` and no prepared layout at 1007 uses the prepared layout at 999 when category is 7. The prepared page for category 7 is not used for that certificate. |

### Done when

`cargo test --lib registration_server::cups::tests::worker` passes, `cargo test --lib registration_server::gate` stays green, and steps 1 through 17 stay green.
