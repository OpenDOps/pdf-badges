# Printing — step 8

Build sequence for [porting.md](porting.md) step 8. The queues, the printer record, the routing, and the badge bytes are [printing.md](printing.md). How the worker shares the core is [design.md](design.md).

This plan adds the `ticket-render` crate and the CUPS client on the server thread. The crate draws a badge to PNG bytes or a 1-bit graphic. The server finds queues, picks one, wraps ZPL when the queue is Zebra or TSC, and submits one IPP job. It does not download CUPS filters, rewrite a PPD, or call `struct_to_pdf`. Sync stores `badge.cfg` and does not draw.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 8 is done when steps 1 through 13 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Layout](#step-1-layout) | The badge document the crate draws | not started |
| [2. Replace](#step-2-replace) | `{{field}}` and the area condition | not started |
| [3. Config](#step-3-config) | `badge.cfg` becomes that document once | not started |
| [4. Page](#step-4-page) | Background and text become a PNG | not started |
| [5. Codes](#step-5-codes) | EAN-13, Code 128, QR, and a photo | not started |
| [6. Graphic](#step-6-graphic) | The 1-bit payload, with no ZPL wrapper | not started |
| [7. Queues](#step-7-queues) | `CUPS-Get-Printers` becomes one `Printer` per queue | not started |
| [8. Devices](#step-8-devices) | A live device marks the queue connected. A new device is installed once | not started |
| [9. Files](#step-9-files) | Settings on the device, routing on the event | not started |
| [10. Pick](#step-10-pick) | Connected, used, category, internet, zone, oldest `last_print` | not started |
| [11. Submit](#step-11-submit) | One `Print-Job`: ZPL raw, or a PNG, options on the job | not started |
| [12. Retry](#step-12-retry) | An aborted job is cancelled and sent again. A stopped queue is resumed | not started |
| [13. Worker](#step-13-worker) | `POST /print` draws, yields, and submits. A failure is the HTTP error | not started |

Shared rules:

- `ticket-render` is the package `modules/ticket-render`. The root `Cargo.toml` depends on it by path. The crate does not depend on `rust-reg`, `lopdf`, or CUPS. Its tests are `cargo test -p ticket-render`. They do not open a socket.
- CUPS lives in `registration_server::cups` in `src/registration_server/cups.rs`. Tests are `cargo test --lib registration_server::cups`. The stand-in is HTTP on `127.0.0.1` and speaks IPP. Tests do not start cupsd and do not open a device.
- A test that writes `base/printer_conf` or `base/{event_id}/pr_{name}.json` uses a temp directory.
- `struct_to_pdf::template::fill` stays on the PDF writer. Badge text does not call it.
- The print worker stays one job on the server thread, yielding between pages, with the gate closed while the job runs. `PRINT_QUEUE_DEPTH` stays 16.
- Sync does not call the crate. Step 3's decoder is what a later `boxapi/getbadge` store calls after the bytes are on disk. This plan does not send `getbadge`.
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
2. The background is drawn first, scaled to that pixel size. Each kept text area is drawn in its box, with its alignment and rotation. The test font is the TTF shipped in the crate. A layout font path that is missing uses that TTF.
3. The crate does not write a file.

### Test scenarios

| Scenario | Assert |
|---|---|
| `png_has_the_page_size` | A 2 by 1 inch layout at 100 dpi is 200 by 100 pixels. |
| `text_is_drawn` | `{{name}}` with `name` = `Ann` changes a pixel inside the text box from the background color. |
| `hidden_area_is_not_drawn` | The same layout with the condition value `false` leaves that pixel on the background color. |

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

The ZPL queue needs the 1-bit graphic. The crate returns that payload. The `^XA` wrapper is step 11.

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

## Step 7. Queues

[Back to summary](#summary)

Search asks cupsd for queues and builds one `Printer` each. Devices and files come in the next steps.

### Work

1. `Cups::search` sends `CUPS-Get-Printers` and reads the queue name, device URI, make and model, state, and whether the queue accepts jobs.
2. Make `zebra technologies` or `tsc` sets `kind` to `zpl`. Any other make sets `raster`. Model `ztc gx430t` and make `evolis` set `dpi` 300. Other ZPL queues set `dpi` 203 and `width_in` 4.1. Evolis sets `width_in` 2.125. A raster queue leaves `width_in` empty.
3. `choices` are the PPD options except `PageSize`. `connected` is false until step 8. `installed` is true because the queue exists.

### Test scenarios

| Scenario | Assert |
|---|---|
| `queues_become_printers` | The stand-in lists a Zebra `ztc gx430t`, a TSC, an Evolis, and one other queue. The four records have kinds `zpl`, `zpl`, `raster`, `raster`, dpi 300, 203, 300, 300, and the ZPL and Evolis widths above. `PageSize` is not in `choices`. |
| `search_sends_get_printers` | The stand-in sees one `CUPS-Get-Printers` and no `Print-Job`. |

### Done when

`cargo test --lib registration_server::cups::tests::queues` passes.

## Step 8. Devices

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

`cargo test --lib registration_server::cups::tests::devices` passes, and step 7 stays green.

## Step 9. Files

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

`cargo test --lib registration_server::cups::tests::files` passes, and steps 7 and 8 stay green.

## Step 10. Pick

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

`cargo test --lib registration_server::cups::tests::pick` passes, and steps 7 through 9 stay green.

## Step 11. Submit

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

`cargo test --lib registration_server::cups::tests::submit` passes, and steps 7 through 10 stay green.

## Step 12. Retry

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

`cargo test --lib registration_server::cups::tests::retry` passes, and step 11 stays green.

## Step 13. Worker

[Back to summary](#summary)

`POST /print` names a registration. The worker on the server thread picks the queue, draws, and submits. The channel still holds one running job.

### Work

1. The request body is the registration id, the category, the internet flag, an optional zone, and an optional queue name. The worker loads the visitor fields from the open event.
2. It picks with step 10. It loads `badge/{category}/layout.json`, or `badge/{category+1000}/layout.json` when the queue has `print_cert`. A missing certificate layout uses category 999, then `-1`, then the smallest non-negative loaded category. The dpi is the queue dpi, lowered so the long side is at most 2400 pixels. A caller may pass a lower cap. The test passes 1200 and sees that cap.
3. One page is one render and one submit. The worker then awaits `yield_now` before a second page. A transliterated copy and a double print are that second page: the server builds the second field map and calls the crate again. The second submit goes to the same queue. The worker does not sleep.
4. The transliterated keys are `company`, `cups_company`, `position`, `cups_position`, `city`, `cups_city`, `name`, `cups_name`, `surname`, `cups_surname`, `patronymic`, `cups_patronymic`.
5. A pick error or a submit error is the HTTP error. No file is written under `genbadges/`.
6. `POST /print/preview` returns the PNG from the crate and does not send `Print-Job`.
7. The gate is held from the start of the job until the last submit returns.

### Test scenarios

| Scenario | Assert |
|---|---|
| `print_submits_one_job` | `POST /print` for a raster queue returns the job id. The stand-in has seen one `Print-Job` with `image/png`. The response is not success when the pick matches nothing. |
| `a_second_page_yields` | A double print sends two jobs to the same queue. The gate is closed across both submits and open after the second returns. |
| `preview_does_not_submit` | `POST /print/preview` returns PNG bytes. The stand-in has seen no `Print-Job`. |
| `certificate_layout` | A queue with `print_cert` and no `badge/1007/layout.json` uses `badge/999/layout.json` when category is 7. |

### Done when

`cargo test --lib registration_server::cups::tests::worker` passes, `cargo test --lib registration_server::gate` stays green, and steps 1 through 12 stay green.
