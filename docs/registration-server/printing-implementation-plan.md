# Printing — step 8

Build sequence for [porting.md](porting.md) step 8. The queues, the printer record, and the badge bytes are [printing.md](printing.md). How a job chooses a device is [printing-routing.md](printing-routing.md), implemented in step 15. What is decoded when the event opens, and which layer is drawn before the visitor, is [printing-preload.md](printing-preload.md). How the worker shares the core is [design.md](design.md).

This plan adds the `ticket-render` crate and the print clients on the server thread. `ticket-render` is the Scala and C badge: `badge.cfg` becomes a layout, then PNG bytes or a 1-bit graphic. `struct_to_pdf` is the other renderer: a YAML page becomes PDF, PNG, BMP, or ZPL. Both are decoded into memory when the event opens and again when sync stores one category. A print draws from that memory. The request path is in [printing.md](printing.md): the worker picks, then draws the one format that queue takes, then submits. Zebra and TSC are ZPL. A raster queue is PNG. A PDF page is one PDF page at that queue's dpi and waits in the step 19 window. The server asks the native print library for the operating-system queues and, in parallel, the IPP tool for IPP and IPPS printers. It joins an IPP queue onto the discovered printer by address. An operating-system queue is submitted through the native library. A printer with no operating-system queue is submitted through the IPP tool. The server does not download CUPS filters or rewrite a PPD. Sync stores the badge files and does not draw.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 8 is done when steps 1 through 19 are done.

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
| [8. Page file](#step-8-page-file) | A YAML page becomes PDF bytes | done |
| [9. Raster file](#step-9-raster-file) | That page becomes PNG and BMP | done |
| [10. ZPL file](#step-10-zpl-file) | That page becomes ZPL | done |
| [11. Prepare](#step-11-prepare) | Both badges are decoded on a switch and when sync stores one category | done |
| [12. Queues](#step-12-queues) | Native queues and discovered IPP printers, joined on the address | done |
| [13. Devices](#step-13-devices) | Device events join the aggregator: a live device marks its queue connected, a new device is installed once | done |
| [14. Files](#step-14-files) | Settings on the device, routing on the event | done |
| [15. Pick](#step-15-pick) | Routing from [printing-routing.md](printing-routing.md): category, certificate, zone | done |
| [16. Submit](#step-16-submit) | One `Print-Job`. The format is raw ZPL, `image/png`, or `application/pdf` | done |
| [17. Retry](#step-17-retry) | An aborted job is cancelled and sent again. A stopped queue is resumed | done |
| [18. Worker](#step-18-worker) | `POST /print` picks, draws the format that queue takes, then submits or appends | done |
| [19. PDF window](#step-19-pdf-window) | Hold the PDF page drawn for the picked queue. One window per printer, or one per zone | done |

Shared rules:

- `ticket-render` is the package `modules/ticket-render`. The root `Cargo.toml` depends on it by path. The crate does not depend on `rust-reg`, `lopdf`, or CUPS. Its tests are `cargo test -p ticket-render`. They do not open a socket.
- The operating-system queue list is [native-print.md](native-print.md), `registration_server::native_print`. Linux and macOS call CUPS. Windows calls the spooler. The build sequence is [native-print-implementation-plan.md](native-print-implementation-plan.md).
- Direct IPP is [ipp-tool.md](ipp-tool.md), the crate `modules/ipp-tool`. The build sequence is [ipp-tool-implementation-plan.md](ipp-tool-implementation-plan.md). Attributes, discovery, and document formats are [ipp.md](ipp.md). The crate does not link CUPS. Tests stand in for DNS-SD and answer IPP on `127.0.0.1`.
- CUPS queue install, the PPD, and a USB `Print-Job` stay in `registration_server::cups` in `src/registration_server/cups.rs`. Tests are `cargo test --lib registration_server::cups`. The stand-in is HTTP on `127.0.0.1` and speaks IPP. Tests do not start cupsd and do not open a device.
- A test that writes `base/printer_conf` or `base/{event_id}/pr_{name}.yml` uses a temp directory. The file is YAML. `class` is `native` or `ipp`.
- `ticket-render` does not call `struct_to_pdf::template::fill`. A YAML badge uses `struct_to_pdf` for its own fill and draw. The two renderers do not share a layout file.
- The print worker is the only caller of pick, draw, and submit. It stays one job on the server thread, yielding between pages. `PRINT_QUEUE_DEPTH` stays 16. Pick reads the aggregator's current list and returns before anything is drawn. The queue's `kind`, dpi, and width choose the bytes: `zpl` is ZPL, `raster` is PNG, and a PDF page is one PDF page at that dpi.
- A ZPL or PNG page is one `Print-Job` as soon as it is drawn. An operating-system queue goes through the native library. A printer with no operating-system queue goes through the IPP tool. A PDF page is not sent as it is drawn. Step 19 holds it. A zone has one window, as [printing-routing.md](printing-routing.md) describes. A job that was not sent to a zone has one window on the printer step 15 chose. `SLIDING_WINDOW_TIME` defaults to 500 milliseconds. The quiet wait does not hold the gate. The flush holds the gate only while that PDF is submitted.
- Sync stores the badge files and does not draw. Step 11 rebuilds that category in memory after the bytes are on disk, and rebuilds every category when the event opens. This plan does not send `getbadge`.
- Step 7 of the sync plan stays green: `cargo test --lib registration_server::sync::tests::queue`. The gate tests stay green: `cargo test --lib registration_server::gate`.

## IPP support

The attribute list, the MIME types, toner, and IPP/IPPS discovery are [ipp.md](ipp.md). The caller is [ipp-tool.md](ipp-tool.md). The operating-system queues are [native-print.md](native-print.md).

Search asks both at once. `native_print::list` returns the queues the OS already has. On Linux and macOS that call is `CUPS-Get-Printers`. On Windows it is `EnumPrinters`. `ipp_tool::discover` browses `_ipp._tcp` and `_ipps._tcp` and keeps the printers that answer `Get-Printer-Attributes`.

A native queue whose device URI is `ipp://` or `ipps://` has a host. That host's address selects the discovered printer. The pair is one record. The queue name stays the native name. Formats, settings, toner, a test page, a file whose type is in `document-format-supported`, and job cancel go through the IPP tool, and that address is queried once.

A discovered printer with no native queue is an extra record. `add` inserts an `ipp://` or `ipps://` URI without a browse. `refresh` browses again and drops a printer that does not answer.

A USB queue and any other non-IPP queue stay native. Step 13 installs those through CUPS. They have no IPP format list. A ZPL or PNG badge is one `Print-Job` in step 16: the native library for an operating-system queue, the IPP tool when the printer has no operating-system queue. A PDF page is drawn for the queue step 15 chose and waits in the step 19 window. The native library and the IPP tool receive that document only after the flush. A zone flush may hand it to another printer in that zone. A window that is not a zone sends it to the same printer.

`set` writes media size, media type, duplex (`sides`), resolution, and color when `printer-settable-attributes-supported` includes that attribute and the value is in the supported list. `cancel` sends `Cancel-Job` and is how a job is interrupted. `jobs` reads the printer's queue.

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

Search follows [printing.md](printing.md) Finding. Each tool sends printers as it finds them. The current list is what a viewer subscribes to. A USB device with no queue is still installed in the next step.

### Work

1. `native_print::watch` sends each operating-system queue as soon as that queue is ready, then `Finished`. On Linux and macOS the queue comes from `CUPS-Get-Printers` and that queue's PPD. On Windows it comes from the spooler, as [native-print-implementation-plan.md](native-print-implementation-plan.md) builds. `list` remains the one-shot call. This step does not grow a second map of queues.
2. `ipp_tool::Catalog::watch` browses `_ipp._tcp` and `_ipps._tcp`, sends each printer that answers `Get-Printer-Attributes`, then `Finished`. A printer already known is queried again on the next watch. One that does not answer is not sent. `details` asks for one URI without a browse and sends that printer, or `Missed`, on the same channel.
3. The aggregator writes each event into the searching list. The current devices list is separate. A device that is not in it is added immediately, and its queue is added in that same step. Subscribers are told. The searching list is not part of the public list.
4. A native `ipp://` or `ipps://` queue looks up its address in the IPP cache. A hit copies formats, settings, and toner and does not ask again. A miss calls `details` on the IPP tool. A USB queue is not passed to the IPP tool. A browse result with no native queue is its own record. The native queue name stays when the two are the same address.
5. When both tools have sent `Finished`, and every `details` ask has been answered, a device the searching list does not have leaves the current devices list, and its queue is removed in that same step. A device that remains updates its queue. Subscribers see each removal and each change. A `Failed` listing does not drop devices or queues. `add` inserts one URI, does not browse, and adds that device and its queue immediately. `refresh` starts another watch of both tools.
6. Make `zebra technologies` or `tsc` on a native queue sets `kind` to `zpl`. Any other native make sets `raster`. Model `ztc gx430t` and make `evolis` set `dpi` 300. Other ZPL queues set `dpi` 203 and `width_in` 4.1. Evolis sets `width_in` 2.125. A raster queue leaves `width_in` empty. `choices` are the PPD options except `PageSize`. A native non-IPP queue has `installed` true and `connected` false until step 13. An IPP printer is `connected` when its attributes have arrived.

### Test scenarios

| Scenario | Assert |
|---|---|
| `queues_become_printers` | The CUPS stand-in lists a Zebra `ztc gx430t`, a TSC, an Evolis, and one other queue. The four native records have kinds `zpl`, `zpl`, `raster`, `raster`, dpi 300, 203, 300, 300, and the ZPL and Evolis widths above. `PageSize` is not in `choices`. |
| `search_sends_get_printers` | The CUPS stand-in sees one `CUPS-Get-Printers` and no `Print-Job`. The IPP stand-in sees the browse and no `Print-Job`. |
| `ipp_queue_reuses_the_address` | A native queue `ipp://192.0.2.10/ipp/print` and a browse result for `192.0.2.10` are one printer. The browse result is cached before the native queue is delivered. `Get-Printer-Attributes` is sent once. The record keeps the native queue name and the IPP format list. |
| `usb_queue_stays_native` | A `usb://` queue is in the list and is not matched to a browse result. |
| `browse_only_printer_is_extra` | A discovered printer with no native queue is a second record. |
| `add_takes_a_uri` | `add("ipps://192.0.2.11/ipp/print")` inserts that printer and does not browse. |
| `refresh_drops_a_silent_printer` | A printer that does not answer `Get-Printer-Attributes` leaves the list. |
| `a_new_printer_is_shown_before_the_search_ends` | One native queue is delivered while the browse is still held. The current list and the subscriber see that printer before the browse finishes. |
| `a_missing_printer_leaves_when_both_finish` | A printer already in the current list stays while one tool has not finished. It leaves once both tools have finished a search that did not find it, and the subscriber is told. |
| `native_ipp_asks_when_the_cache_misses` | A native `ipp://` queue whose address is not cached is given to the IPP tool. The stand-in sees `Get-Printer-Attributes`. The record keeps the native name and gains the format list. |
| `native_watch_sends_each_queue_then_finished` | The first queue is on the channel while the next queue's PPD is still held. `Finished` follows the last queue. |
| `ipp_watch_sends_each_printer_then_finished` | One browse result is delivered before the next answer is released. `_printer._tcp` is not queried. `Finished` follows. A URI that does not answer is `Missed`. |
| `an_ipp_printer_is_shown_before_native_finishes` | The subscriber and the current list see the browse result while the native listing is still held. |
| `a_finished_search_updates_the_same_printer` | The same queue is found again with a new make. The current list still has one record. The subscriber is told it was updated. |
| `the_same_printer_is_not_added_again` | A refresh that finds the same queue does not add a second record and does not emit `Added`. |
| `add_shows_the_printer_while_the_listing_is_held` | `add` puts the printer on the current list while both listings are still held, and browse has not run. |
| `usb_is_not_passed_to_the_ipp_tool` | A USB queue is listed. The IPP stand-in sees no `Get-Printer-Attributes`. |
| `a_failed_listing_keeps_the_current_list` | The native listing fails on refresh. `wait` returns an error. The printer that was already current stays. |
| `queues_follow_the_current_devices` | A new device adds a queue in the same step. A device that stays keeps its queue until both tools finish. A device that then leaves removes its queue in that same step. The queues are exactly the current devices. |

### Done when

`cargo test --lib registration_server::cups::tests::queues` and `cargo test -p ipp-tool` pass, and steps 1 through 11 stay green.

## Step 13. Devices

[Back to summary](#summary)

Device events join the step 12 aggregator. They do not start a second search and they do not replace the current devices list. A live device marks its queue connected. A USB device with no queue is installed once, and that queue is added with the device. This step does not browse IPP. An `ipp://` or `ipps://` queue keeps the `connected` value step 12 set from `Get-Printer-Attributes`.

The device watch is the CUPS client, so it compiles on Linux and macOS with the queue watch. A Windows build does not send `CUPS-Get-Devices`.

### Work

1. `native_print` sends each `CUPS-Get-Devices` device as soon as that device is read, then `Finished`. The aggregator treats that finished event as a third listing, beside the queue watch and the IPP watch. Step 12 does not drop a device until all three have finished. A `Failed` device listing removes nothing.
2. A device event whose URI equals a queue already on the current devices list sets that queue `connected` immediately. The subscriber is told. The queue is not added again.
3. When the device listing finishes, a non-IPP queue with no matching device stays on the current devices list with `connected` false. Its queue stays. The device listing is not the step 12 removal.
4. A device with no queue enters the current devices list immediately, with `installed` false. `CUPS-Get-PPDs` is asked for that make and model. Zebra uses `zebra.ppd` when the stand-in lists it. Evolis uses `evolis-zeniusE.ppd` when it lists it. One `CUPS-Add-Modify-Printer` installs the queue. The name is make, model, and the USB serial: spaces become `_`, and the serial is cut at `&`. When the install succeeds, that queue is added with the device and `installed` becomes true. The subscriber sees the device added, then the queue updated.
5. A make with no PPD stays on the current devices list with `installed` false. No `Add-Modify` is sent. No queue is created.
6. A later watch sees the queue the install created. The device matches that queue. No second `Add-Modify` is sent.
7. A stopped queue, or one that is not accepting jobs, is resumed and accepted the first time it is adopted. A later watch does not resume it again. Saved option values are not sent.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_live_device_is_connected` | Two queues are already current. One device event matches the first URI while the device listing is still held. That queue becomes `connected` and the subscriber is told before the listing finishes. After `Finished`, the unmatched queue is still current with `connected` false. Neither queue was removed. |
| `a_missing_device_keeps_the_queue` | The device listing finishes with no device for a current queue. The queue stays. `connected` is false. |
| `a_new_device_is_installed_once` | A USB device `usb://Zebra Technologies/ZTC?serial=ABC&extra` enters the current devices list before `Add-Modify`. One `Add-Modify` uses the queue name `Zebra_Technologies_ZTC_ABC` and `zebra.ppd`. The queue is then on the list with that device and `installed` true. The next watch sends no `Add-Modify`. |
| `no_ppd_is_not_installed` | A device whose make is absent from `CUPS-Get-PPDs` is added to the current devices list with `installed` false. The stand-in sees no `Add-Modify`. It is still there after the device listing finishes. |
| `a_stopped_queue_is_resumed_once` | A stopped queue is resumed and accepted the first time it is adopted. The next watch sends neither operation. No saved option is sent. |

### Done when

`cargo test --lib registration_server::cups::tests::devices` passes, and step 12 stays green.

## Step 14. Files

[Back to summary](#summary)

Settings belong to the device. Routing belongs to the open event. Both files are YAML, and both carry `class`. Search loads them onto the current record when that device is accepted. It does not clear the list.

`class` is `native` or `ipp`. An operating-system queue is `native`, including a queue whose URI is `ipp://` or `ipps://`. A printer with no operating-system queue is `ipp`.

### Work

1. `base/printer_conf/{name}.yml` is `values`, plus `class`. A missing file is written from the PPD defaults on that search. ZPL defaults are the Scala defaults: tracking `Web`, media `Direct`, mode `Cutter`, speed `10`, darkness `0`, shift `0`, top `0`, tear-off `0`. Evolis default is `InkType` `Black`. `class` is taken from the current device.
2. `base/{event_id}/pr_{name}.yml` is the routing record, plus the same `class`: sector name, used, zone, category, internet, second name, print certificate, auto print, no postscript. A missing file loads as used, with an empty sector name and the flags false. `last_print` is not in the file. It starts at 0.
3. `save_routing` writes every field, including when second name is empty, and writes `class`.
4. Search does not send the file values to cupsd.

### Test scenarios

| Scenario | Assert |
|---|---|
| `missing_settings_are_written` | A Zebra queue with no settings file gets `base/printer_conf/{name}.yml`. `class` is `native`. Darkness is `0` and speed is `10`. The stand-in sees no `Print-Job` and no option-setting operation. |
| `an_ipp_printer_is_class_ipp` | A printer added by URI, with no operating-system queue, gets `class: ipp` in both YAML files. |
| `routing_roundtrip_without_a_second_name` | A record with empty second name, `print_cert` true, and `auto_print` true is saved and loaded with those three fields intact. `class` round-trips. |
| `missing_routing_is_used` | A queue with no `pr_{name}.yml` loads used, with no zone and no category. |

### Done when

`cargo test --lib registration_server::cups::tests::files` passes, and steps 12 and 13 stay green.

## Step 15. Pick

[Back to summary](#summary)

This step implements [printing-routing.md](printing-routing.md). A print names a visitor category, an internet flag, an optional certificate attribute, an optional zone, and an optional queue name. Pick reads the aggregator's current list and returns one connected, installed, used queue, or an error. It does not draw and it does not open a socket. The worker in step 18 is the caller. The returned queue carries `kind`, dpi, and width. Step 18 draws ZPL, PNG, or one PDF page from that queue.

### Work

1. An explicit name returns that queue when it is connected and installed.
2. A job with the certificate attribute returns a printer with `print_cert`. The category printer is not that result. Several certificate printers in the job's zone use the zone rule.
3. A category returns the used printer bound to that category. A printer with no category stays only when no used printer has the category. The internet flag uses the same rule.
4. A zone returns one printer whose `zone` is that zone. `round_robin` takes the oldest `last_print`. `fewest_jobs` takes the printer whose device queue has fewer jobs, and an equal count uses the oldest `last_print`. A missing rule is `round_robin`.
5. An empty candidate list is an error. A queue with `used` false is not a fallback. `auto_print` and `no_postscript` are not read here. `print_cert` is the route in item 2. Step 18 uses that flag for the certificate layout, and uses `kind`, dpi, and width for the bytes.
6. A successful pick sets `last_print` on that queue.

### Test scenarios

| Scenario | Assert |
|---|---|
| `explicit_name_wins` | The named queue is returned even when another queue is older. |
| `category_goes_to_its_printer` | Category 7 returns the printer bound to category 7. A printer with no category is not returned while that printer is used. The internet flag uses the same rule. |
| `a_certificate_goes_to_the_certificate_printer` | A job with the certificate attribute returns the printer with `print_cert`, not the printer bound to the visitor category. |
| `a_zone_round_robins` | Zone 2 with `round_robin` returns the printer in that zone with the older `last_print`. That `last_print` changes. A printer bound to zone 1 is not returned. |
| `a_zone_picks_the_shorter_queue` | Zone 2 with `fewest_jobs` returns the printer whose device queue has fewer jobs. An equal count returns the older `last_print`. |
| `nothing_matching_is_an_error` | Every queue is disconnected, or every matching queue has `used` false. The error names no queue. `last_print` is unchanged. |

### Done when

`cargo test --lib registration_server::cups::tests::pick` passes, and steps 12 through 14 stay green.

## Step 16. Submit

[Back to summary](#summary)

One page is one `Print-Job`. The page selects the document format: raw ZPL, `image/png`, or `application/pdf`. Postscript is not sent. This step does not pick and does not draw. A PDF page still waiting in the step 19 window is not sent here. The flush calls this same `Print-Job`. Options travel on the request.

### Work

1. A `zpl` queue sends the graphic from step 6 wrapped as `^XA`, the seven Zebra settings (`^PR`, `^MD`, `^MM`, `^LS`, `^LT`, `~TA`), `^FO` centered with `width_in`, and `^XZ`. The document format is raw. `zeMediaTracking`, `MediaType`, and `zePrintMode` are also IPP options. The dpi and the 4.1 inch width are the ones on that queue.
2. A `raster` queue sends the PNG from step 4. The document format is `image/png`. Evolis adds `InkType` and the card size. The media size is the layout's inch width and height. `no_postscript` does not change the format.
3. An operating-system queue (`class` `native`) sends that job through the native library. A printer with no operating-system queue (`class` `ipp`) sends the same body through the IPP tool. The CUPS stand-in sees no `Print-Job` for that IPP printer.
4. A PDF page sends those bytes. The document format is `application/pdf`. `no_postscript` does not select Postscript.
5. The stand-in answers `job-state` completed. The call returns the job id.

### Test scenarios

| Scenario | Assert |
|---|---|
| `zpl_job_is_raw` | The body starts with `^XA`, contains `^PR10` and `^MD0` for the default settings, contains the graphic, and ends with `^XZ`. The document format is raw. The IPP options include `zeMediaTracking=Web`. |
| `raster_job_is_png` | An Evolis queue with `no_postscript` true sends `image/png`, `InkType=Black`, and a media size taken from the layout. The body is a PNG. The stand-in sees no request that writes a PPD. |
| `an_ipp_printer_uses_the_ipp_tool` | A raster printer with no operating-system queue sends `image/png` through the IPP tool. The CUPS stand-in sees no `Print-Job`. |
| `a_pdf_page_is_pdf` | A PDF page sends `application/pdf` and the PDF bytes. The request does not use `application/postscript`. |

### Done when

`cargo test --lib registration_server::cups::tests::submit` passes, and steps 12 through 15 stay green.

Done. `Search::submit` sends one `Print-Job`. A graphic on a `zpl` queue is raw `application/octet-stream`. A PNG is `image/png`. A PDF page is `application/pdf`. An operating-system queue uses the native library. A printer with no operating-system queue uses the IPP tool.

## Step 17. Retry

[Back to summary](#summary)

Completion is `Get-Job-Attributes`. A failed job is cancelled and sent again, up to ten times. The same retry covers a step 16 `Print-Job` and the PDF the step 19 flush submits.

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

Done. After `Print-Job`, `Get-Job-Attributes` reads `job-state`. `aborted` is cancelled and sent again, up to ten times. A stopped queue is resumed before the job, and again when a try fails because the queue is stopped.

## Step 18. Worker

[Back to summary](#summary)

`POST /print` names a registration. The worker on the server thread is the only caller. It picks, then draws the one format that queue takes, then submits or appends. The channel still holds one running job.

### Work

1. The request body is the registration id, the category, the internet flag, an optional zone, and an optional queue name. The worker loads the visitor fields from the open event, then calls pick.
2. Pick is step 15. It reads the aggregator's current list. The searching list stays inside the aggregator. The returned queue carries `kind`, dpi, width, `print_cert`, and `class`.
3. The draw uses that queue and reads the prepared memory. It does not read `badge.cfg`, `layout.json`, a page file, a font file, or an image file. `print_cert` selects the certificate layout: category plus 1000, then 999, then `-1`, then the smallest non-negative prepared category. The prepared page for the visitor category is not used for that certificate. The dpi is the queue dpi, lowered so the long side is at most 2400 pixels. A caller may pass a lower cap. The test passes 1200 and sees that cap. The worker draws only the format that queue takes. A `zpl` queue draws ZPL at that dpi and width: step 10 when the category has a prepared page, otherwise the 1-bit graphic wrapped as `^XA` … `^XZ` with that queue's Zebra settings. A `raster` queue draws a PNG at that dpi: step 9 when a prepared page is present, otherwise ticket-render. A PDF page draws one PDF page at that dpi from step 8. A layout-only category is ZPL or PNG, from `kind`.
4. A ZPL or PNG page is one submit through step 16. An operating-system queue uses the native library. A printer with no operating-system queue uses the IPP tool. A PDF page is one append on the step 19 window: the zone's window when the job was routed to a zone, and that printer's window otherwise. This step does not send the PDF. The worker then awaits `yield_now` before a second page. A transliterated copy and a double print are that second page. The second ZPL or PNG submit goes to the same queue. The second PDF page appends to the same window. The worker does not sleep.
5. The transliterated keys are `company`, `cups_company`, `position`, `cups_position`, `city`, `cups_city`, `name`, `cups_name`, `surname`, `cups_surname`, `patronymic`, `cups_patronymic`.
6. When no connected printer is picked, the worker draws a PNG and a one-page PDF at 720 dpi, lowered until the long side is at most 2400 pixels, and does not send `Print-Job`. The stem is `{category}_{name}_{surname}_{patronymic}_{company}`. The PNG is `{event}/genbadges-png/{stem}.png`. The PDF is `{event}/genbadges-pdf/{stem}.pdf`. Both directories are created when missing. `isPrintCert` is false for both files. A category with a prepared page writes the PNG from step 9 and the PDF from step 8. A category with only a prepared layout writes the PNG from step 4 and a one-page PDF of that image at the layout's inch size. Transliteration writes the `-tr` pair. A double print without transliteration writes the `-d` pair. The response names both paths and has no job id.
7. A submit error after a queue was picked is the HTTP error. That failure writes neither `genbadges-png/` nor `genbadges-pdf/`.
8. `POST /print/preview` picks so the dpi and the certificate layout match, returns the PNG for that queue, and does not send `Print-Job`. A `zpl` preview is still the PNG, not the `^XA` wrapper.
9. The gate is held from the start of the job through the draw. A ZPL or PNG job holds it until the last immediate submit returns. A PDF job holds it through the append and releases it for the quiet wait. A no-printer write holds the gate until both files are written.

### Test scenarios

| Scenario | Assert |
|---|---|
| `print_submits_one_job` | `POST /print` for a raster queue returns the job id. The stand-in has seen one `Print-Job` with `image/png`. The PDF window stays empty. |
| `a_zpl_queue_submits_raw` | `POST /print` for a Zebra queue sends one raw body that starts with `^XA` and ends with `^XZ`. The PDF window stays empty. |
| `no_printer_writes_genbadges` | No connected queue writes `{event}/genbadges-png/{category}_{name}_{surname}_{patronymic}_{company}.png` and `{event}/genbadges-pdf/{category}_{name}_{surname}_{patronymic}_{company}.pdf`, and sends no `Print-Job`. A prepared page uses the step 9 PNG and the step 8 PDF. A layout-only category uses the step 4 PNG and a one-page PDF of that image. The response names both paths and has no job id. |
| `a_second_page_yields` | A double print of a raster queue sends two jobs to the same queue. The gate is closed across both submits and open after the second returns. A second PDF page appends to the same window and the stand-in sees no `Print-Job`. |
| `preview_does_not_submit` | `POST /print/preview` returns PNG bytes. The stand-in has seen no `Print-Job`. A category with a prepared page returns the PNG from step 9. A category with only a prepared layout returns the PNG from `ticket-render`. A Zebra queue still returns the PNG. |
| `certificate_layout` | A queue with `print_cert` and no prepared layout at 1007 uses the prepared layout at 999 when category is 7. The prepared page for category 7 is not used for that certificate. |

### Done when

`cargo test --lib registration_server::cups::tests::worker` passes, `cargo test --lib registration_server::gate` stays green, and steps 1 through 17 stay green.

Done. `Search::print` is the `POST /print` job. It picks from the current list, draws ZPL, PNG, or one PDF page from the prepared memory, and submits a ZPL or PNG page at once. A PDF page is appended and is not sent here. No connected printer writes `genbadges-png` and `genbadges-pdf` and sends no `Print-Job`. `POST /print/preview` returns the PNG and does not submit.

## Step 19. PDF window

[Back to summary](#summary)

A PDF page is already drawn for the queue pick chose, at that queue's dpi, then held. The hold is in front of the native library and the IPP tool. ZPL and PNG never enter it. A zone has one window, as [printing-routing.md](printing-routing.md) describes. A job that was not sent to a zone has one window on the printer step 15 chose. The window closes when that destination has been quiet for `SLIDING_WINDOW_TIME`. The flush builds one PDF and sends that document.

### Work

1. `SLIDING_WINDOW_TIME` is 500 milliseconds when unset. A test passes the duration. The clock is the one the test supplies.
2. A job routed to a zone appends to that zone's window. Each page was drawn for the printer pick chose when that page arrived. The zone has one `last_job_at`. The flush runs the zone rule again and sends the one PDF to the printer chosen then, which may be another printer in that zone. A job that was not routed to a zone uses the printer queue picked in step 15, and the flush sends the document to that same printer. Two such printers do not share pages, and each has its own `last_job_at`.
3. A PDF page is appended to that window's list and `last_job_at` becomes the time of this page. A page that arrives while `last_job_at + SLIDING_WINDOW_TIME` is still ahead of now extends the same document. The first page and a second page inside the window are one two-page PDF. A ZPL page and a PNG page are not appended. They submit in step 16 as they are drawn.
4. The list is flushed when `last_job_at + SLIDING_WINDOW_TIME` is before now. The flush concatenates the held pages into one PDF, in arrival order, and sends that document. An IPP printer receives it from the IPP tool as `application/pdf`. A native queue receives it from the native library as `application/pdf`. The list is then empty.
5. A later page for a printer whose window has already flushed starts a new document. It does not join the document that was sent.
6. Rendering and appending hold the gate. The wait does not. The flush holds the gate only while the finished PDF is submitted. A printer with no connected queue does not enter this window. Step 18 writes that badge to `genbadges-pdf/` and `genbadges-png/` immediately.

### Test scenarios

| Scenario | Assert |
|---|---|
| `two_pages_inside_the_window_are_one_pdf` | Printer A receives a page at t and another at t + 400 milliseconds. The window is 500 milliseconds. One document is sent, with two pages, and the send time is after t + 400 milliseconds + the window. The native library and the IPP tool see no job before that flush. |
| `a_late_page_starts_a_new_document` | The second page arrives at t + 500 milliseconds. The first document is one page and has already been sent. The second page is a new document. |
| `printers_do_not_share_a_window` | A page for printer A and a page for printer B, neither sent to a zone, arrive 10 milliseconds apart. Each document is one page. A's `last_job_at` does not move B's. |
| `a_zone_has_one_window` | A page routed to printer A in zone 2 and a page routed to printer B in zone 2 arrive 10 milliseconds apart. Each page was drawn for the printer chosen when it arrived. One document is sent, with two pages. The flush chooses one printer in zone 2 and may send the document to the other printer. |
| `the_window_slides_with_the_last_page` | Pages at t, t + 400 milliseconds, and t + 800 milliseconds, with a 500 millisecond window, are one three-page document. The flush is after t + 800 milliseconds + the window. |
| `zpl_and_png_are_not_held` | A ZPL job and a PNG job are submitted at once. The PDF window for that printer stays empty. |

### Done when

`cargo test --lib registration_server::cups::tests::pdf_window` passes, `cargo test --lib registration_server::gate` stays green, and steps 1 through 18 stay green.

Done. A PDF page waits on the printer's window, or on the zone's one window. `SLIDING_WINDOW_TIME` is 500 milliseconds. The flush sends one `application/pdf` after the destination has been quiet for that long. ZPL and PNG are not held. A page that arrives once the window has closed starts a new document.
