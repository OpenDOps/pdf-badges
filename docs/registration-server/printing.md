# Printing

How the Scala client finds a printer through CUPS, draws a badge, and sends the job, and what the Rust process should do instead. [porting.md](porting.md) step 8 is this work. The build sequence is [printing-implementation-plan.md](printing-implementation-plan.md). How a print job shares the core with local HTTP is [design.md](design.md). The sync queue does not render and does not print.

The Scala sources are `print/PrinterFactory.java`, `print/Printer.java`, `print/ZPLPrinter.java`, `print/ZebraPrinter.java`, `print/DefaultPrinter.java`, `print/EvolisPrinter.java`, `PrintHelper.scala`, and `RegBoxHelpers.scala` (`initPrint`, `renderBadge`, `getPrinterName`). Drawing lives in the separate tree `ticketrender`: Java `OutputAreasHolder` plus the JNI library `libcairobadge.so`.

## What the Scala client does

`PrintHelper.initPrint` runs when the event data is loaded. It asks CUPS which printers are connected, builds one object per queue, and restores that queue's saved options.

### Search

On Linux the search is two commands.

`lpinfo -v` lists device URIs. The client keeps lines whose URI starts with `usb://` or `ipp://`.

`lpstat -s` lists queues cupsd already has. The client matches an English line (`device for NAME: URI`) and a Russian line (`устройство для NAME: URI`). A queue whose URI matches a device from `lpinfo`, after replacing spaces, dashes, and a `Series_` fragment, is marked connected. A device that matched no queue is installed (below) and added under a name built from make, model, and serial.

The URI parser accepts `usb://MAKE/MODEL?serial=SERIAL`, `ipp://NAME.something/`, and `implicitclass://NAME/`. An `ippusb` URI is rewritten toward a matching `ipp://` device before that parse.

Windows uses `winprint` (`PrintersManager.listPrinters`) and a `WinPrinter`. The venue device is Linux. That path is recorded here so the CUPS path is the one step 8 ports.

### Init through CUPS

A new queue is installed with `lpadmin -p NAME -v URI` and a PPD. The PPD is chosen in this order:

1. A path the subclass hard-codes (`./ppd/zebra.ppd`, `./ppd/evolis-zeniusE.ppd`).
2. `./ppd/{make}-{model}.ppd` if that file is already on disk.
3. `lpinfo --make-and-model "MAKE MODEL" -m`, then `lpadmin -m` that model.
4. A tarball `{make}-{model}.tar.gz` downloaded from `{app.update.url}printer_filters/{os}/{arch}/`, unpacked, and symlinked into the CUPS filter directory and `usr/lib`. The PPD from that tarball is then passed to `lpadmin -P`.

`cupsenable` and `cupsaccept` run after a successful install. `lpc status NAME` decides whether the install worked.

`lpoptions -p NAME -l` reads the PPD options. `lpoptions -p NAME -o ...` writes the chosen ones back onto the queue. Zebra sets `zeMediaTracking`, `MediaType`, `zePrintMode`, a custom `PageSize`, and `Resolution`. Evolis sets `InkType=Black`. A default printer copies every PPD option except `PageSize` into a settings map, then rewrites `/etc/cups/ppd/{name}.ppd` so `*DefaultPageSize` and the other defaults match.

Saved values live in a JSON file under the printer-conf directory, one file per queue name. On the next init those values are applied with `setOptions` again.

Make selects the class:

| make | class | what a job is |
|---|---|---|
| Zebra Technologies | `ZebraPrinter` | ZPL, sent raw |
| TSC | `ZebraPrinter` | same ZPL path. `TSCPrinter` is not constructed |
| Evolis | `EvolisPrinter` | PostScript. Card size is fixed at 2.125 by 3.370 inches. DPI 300 |
| anything else | `DefaultPrinter` | PostScript, or a BMP passed through a `*cupsFilter` |
| Godex | commented out | not constructed |

A Zebra model `ztc gx430t` uses 300 DPI. Other ZPL queues use 203 DPI and a 4.1 inch width.

### Send

`printBadge` picks a queue, renders, converts, then submits.

Routing is `getPrinterName`. An explicit printer name wins. Otherwise the client keeps queues with `used`, whose category matches when any queue has that category, and whose internet flag matches when any queue has that flag. A requested zone prefers a queue bound to that zone. Among the matches, the queue with the oldest `lastPrint` is taken. Each queue's routing is `base/{event}/pr_{name}.json`: sector name, used, zone, category, internet, second name, print certificate, auto print, skip PostScript.

The layout is `badge/{category}/badge.cfg`, or `badge/{category+1000}/badge.cfg` when the queue prints a certificate. `renderBadge` sets the queue's paper size from the layout's inch size, lowers DPI until the long side is at most 2400 pixels (1200 on the tiny build), and calls `OutputAreasHolder.render`. The C renderer writes `/tmp/badges/{id}.zpl`, `.ps`, `.png`, or `.bmp`.

`convert` runs `resources/convert.sh` when the PPD has a raster filter. ZPL skips that.

The send is `lp`. ZPL is `lp -o raw -d NAME /tmp/badges/{id}.zpl`. A raster filter uses `lp -o raw` on the `.raw` file. PostScript is `lp -d NAME` on the `.ps` file. `rawPrint` reads `request id is NAME-JOB (1 file(s))`, then polls `lpstat -l -o NAME` until the line `Alerts: job-printing` disappears. The line `Status: Unable to send data to printer.` cancels the job with `lprm` and retries, up to ten times, with a 100 ms sleep between polls.

One single-thread pool per category renders. One single-thread pool per queue sends. A failed ZPL send used to keep the badge and retry it later. That retry list is commented out. A thrown error in `printBadge` renders a PNG into `genbadges/` with a disconnected printer and returns no queue.

ZPL options are not CUPS options at send time. `getExtraParams` formats `^PR` speed, `^MD` darkness, `^MM` mode, `^LS` shift, `^LT` top, and `~TA` tear-off. The C renderer writes those lines into the ZPL file between `^XA` and the graphic.

### What drawing does

`ticketrender` loads `libcairobadge.so` (Cairo, FreeType, libqrencode). `badge.cfg` is a Java object stream, `serialVersionUID` 5, written by `OutputAreasHolder.writeObject` and `OutputArea.writeObject`. `boxapi/getbadge` returns that stream and `bg.png` in one octet-stream body. Sync stores both under `badge/{category}/`.

An area is text, a barcode, or a photo. Text may contain `{{field}}`. A condition string can hide an area. Alignment, rotation, color, font family, weight, and style are on the area. Barcodes drawn in the C file are EAN-13 and Code 128. QR uses libqrencode. The background is `bg.png`.

The ZPL writer thresholds the Cairo pixels to 1-bit, hex-encodes a `~DG` graphic, then wraps `^XA`, the extra-parameter lines, `^FO` centered on the printer width, and `^XZ`. PostScript is a hand-written `image` operator over the same pixels, 50 rows at a time. PNG and BMP are the other two writers. A Cairo PostScript surface is created for the `ps` format and then abandoned. The file that is printed is the hand-written one.

## ticket-render

Drawing becomes its own crate, `modules/ticket-render`. The registration server links it. The PDF tooling crate stays a different module: it fills a PDF template from a table and can run as a gRPC service. A badge is a small layout drawn to a thermal bitmap. The venue binary does not start the PDF service to print a badge.

The crate does not speak CUPS, does not shell out, and does not write `/tmp/badges`. It takes a layout, the visitor fields, a DPI, and a target (PNG or a 1-bit graphic) and returns bytes.

```text
badge.cfg + bg.png
        │
        │  decode once, store layout JSON
        ▼
ticket-render          text, barcode, photo, background
        │
        │  PNG, or a 1-bit graphic
        ▼
registration-server   wraps ZPL, or submits the PNG to CUPS
```

The layout JSON is the crate's document. The fields follow the Java `writeObject` stream: page width and height, dots per point, then each area's box, alignment, rotation, color, text, condition, type, and either a barcode type or the font and text parts. `serialVersionUID` other than 5 is rejected. The decoder is fixed against captured `badge.cfg` files, because the writer and the reader disagree on the text-part font-family count when that count differs from the area's. Fixtures say which bytes are real.

Render reads the JSON and `bg.png`. It replaces `{{field}}` the way `OutputArea.calculateText` does: one pass, the name lowercased, a missing name left blank. That is the whole substitution. There are no sections, no partials, and no HTML escapes. `struct_to_pdf::template::fill` escapes `&`, `<`, and `>` and looks the name up in the original case, so a badge that called it would print those escapes. ticket-render does its own pass and does not call that function. An area's condition is a separate lookup of `ifSt` in the same value map. An empty result, `false`, or `0` skips the area. An empty `ifSt` shows it. The crate then wraps text and draws. EAN-13, Code 128, and QR stay, with tests on known payloads. Photo areas load the visitor image the caller passes in. The 1-bit graphic is the `~DG` payload only. Speed, darkness, media mode, shift, top, and tear-off are Zebra settings. The registration server writes those `^XA` lines when it builds the ZPL job. The crate does not format them.

Transliteration and a second copy of the badge stay in the registration server. The server builds the second field map and calls the crate again. The crate draws the map it is given.

`renderQrCode` becomes a function on the same crate. Callers pass the string and the pixel size and get PNG bytes.

## What to change

The CUPS queue is the right place to send a job. The way the Scala client finds that queue, prepares it, and draws the badge is what step 8 replaces.

### Finding

Search runs at startup and when the operator asks to refresh. It asks the local cupsd over IPP. `CUPS-Get-Printers` returns every queue: name, device URI, make and model, state, and whether the queue accepts jobs. `CUPS-Get-Devices` returns USB and IPP devices cupsd can see right now. The two lists meet on the device URI string. Both operations return that URI, so the match is exact. The English and Russian `lpstat` lines, the `ippusb` rewrite, and the `Series_` trim go away with the shell commands.

A queue whose URI is in the device list is connected. A queue whose device is absent stays in the list with `connected` false, so its routing file is still there when the cable comes back. A pick skips it.

A device with no queue is installed once, with `CUPS-Add-Modify-Printer`, using a PPD name `CUPS-Get-PPDs` already lists for that make and model. Zebra still prefers the PPD shipped with the device image. Evolis still prefers its shipped PPD. The new queue's name is the Scala name: make, model, and the USB serial, spaces turned into `_`, serial cut at `&`. An existing queue keeps the name cupsd already has. Routing files are keyed by that name, so a printer the old client installed still finds `pr_{name}.json`.

The process does not download a filter tarball, does not symlink into the CUPS filter directory, and does not fetch a PPD from OpenPrinting. Filters belong to the device image. A make with no PPD on the device stays a device with `installed` false. The operator sees it. A pick skips it.

A stopped queue, or one that rejects jobs, is resumed when it is first adopted, and again when a job fails because the queue is stopped. Adopting a queue does not push saved options into cupsd.

### The printer record

Finding builds one `Printer` per queue, plus one for each device that could not be installed. The record is memory. Cupsd owns the queue. Two JSON files own what the operator set.

| field | source | kept |
|---|---|---|
| `name` | queue name | the id. Routing and settings files use it |
| `device_uri` | queue, or the device when there is no queue | refreshed on each search |
| `make`, `model` | `printer-make-and-model`, or the `usb://` URI for a new device | refreshed on each search |
| `state`, `accepting` | queue attributes | refreshed on each search |
| `connected` | the device URI was in `CUPS-Get-Devices` | refreshed on each search |
| `installed` | the queue exists | false only for a device with no PPD |
| `kind` | make | `zpl` for Zebra and TSC, `raster` otherwise |
| `dpi` | make and model | 300 for `ztc gx430t` and Evolis, 203 for other ZPL, 300 for every other raster queue |
| `width_in` | kind | 4.1 for ZPL, 2.125 for Evolis. A raster queue takes the layout width when the job is built |
| `choices` | PPD option list | name to allowed values. `PageSize` is left out. Media size comes from the layout on the job |
| `values` | `base/printer_conf/{name}.json` | the operator's saved options. Written when the operator saves. Applied as IPP options on the job |
| `routing` | `base/{event_id}/pr_{name}.json` | this event only. See below |

`values` survive an event switch. They describe the printer on this device. `routing` does not. It describes how this event uses that printer. Search loads both after the IPP lists return. It replaces the in-memory map as a whole, so a pick never observes a map that was cleared and not yet refilled. `getPrintersData` in the Scala client clears that map on every call.

ZPL `values` are `zeMediaTracking`, `MediaType`, `zePrintMode`, `Speed`, `Darkness`, `zpl_label_shift`, `zpl_label_top`, `zpl_tear_off`. The first three also go out as IPP options. All seven are written into the `^XA` block by the registration server. Evolis stores `InkType`. A raster queue stores the PPD options the operator changed. A missing settings file is written on the first search, from the PPD defaults, the same way `initPrint` writes `{name}.json` when the file is absent.

`choices` are the allowed set the operator UI shows. They are not a class hierarchy. `Printer`, `ZPLPrinter`, `ZebraPrinter`, `DefaultPrinter`, and `EvolisPrinter` were that hierarchy. `kind` plus `values` replaces them.

### Options travel on the job

`lpoptions` writes the queue default, and `DefaultPrinter.setOptions` rewrites `/etc/cups/ppd/{name}.ppd` in place. Two badges that need different page sizes fight over that file. Step 8 leaves the PPD untouched. Media size, resolution, Zebra `zeMediaTracking`, `MediaType`, `zePrintMode`, and Evolis `InkType` are IPP options on that `Print-Job`. The layout's inch width and height are the media size for that job.

Saved settings stay `base/printer_conf/{name}.json`, one file per queue, shared by every event. Search reads the file into `values`. The job sends those values. Search does not push them into cupsd.

### Two kinds of job, one sender

The class tree (`Printer`, `ZPLPrinter`, `ZebraPrinter`, `DefaultPrinter`, `EvolisPrinter`, `GODEXPrinter`, `TSCPrinter`, `WinPrinter`) collapses to two job kinds.

| kind | queues | body | IPP document format |
|---|---|---|---|
| ZPL | Zebra, TSC | `^XA`, the saved Zebra settings, the graphic from ticket-render, `^XZ` | raw |
| raster | Evolis, every other installed queue | PNG from ticket-render | `image/png` |

CUPS filters the PNG with the queue's PPD. The registration process does not write PostScript, does not write BMP, and does not run `convert.sh`. `no_postscript` was the switch that forced a raw raster into a filter. A PNG job uses the filter the PPD already declares, so that flag is not a second rendering path. It remains on the routing record so an old `pr_{name}.json` still loads. The sender ignores it.

Evolis is the raster kind plus its card media and `InkType=Black`. It does not need its own type. Godex and the Windows printer are not constructed.

### One worker sends

`PrintHelper` keeps a render pool per category and a send pool per queue, so several badges occupy the core together. The skeleton already has one worker on the server thread (`print_queue.rs`). That worker runs one job. Up to `PRINT_QUEUE_DEPTH` (16) jobs wait on the channel. Between pages the worker awaits `yield_now`, and the gate stays closed while the job runs, as [design.md](design.md) describes.

A job is: resolve the queue, render one page, submit it, yield. A second page (the transliterated copy, or the double print) is the next yield point, then another submit to the same queue. Render that takes a library with no yield point uses the server runtime's blocking pool, whose size is 1.

Submit is IPP `Print-Job` to that queue. Completion is `Get-Job-Attributes`: `job-state` and `job-state-reasons`. A job that ends `aborted`, or a printer state that cannot accept data, is cancelled with `Cancel-Job` and retried up to ten times. The retry waits on the job state. It does not poll `lpstat` for an English status line.

A send that still fails is an error on the HTTP response. The client does not write a PNG into `genbadges/` and return success. A preview PNG is a separate call that asks ticket-render for PNG bytes and does not open a socket to cupsd.

### Routing

Routing is a field on `Printer`, stored in `base/{event_id}/pr_{name}.json`. A missing file loads as used, with an empty sector name and no zone, category, internet flag, or second name. `print_cert` and `auto_print` load false. `no_postscript` loads false and stays on the record so an old file still parses. The sender ignores it.

| field | meaning |
|---|---|
| `sector_name` | label printed on a test badge |
| `used` | a pick may choose this queue |
| `zone` | set when the queue belongs to one control zone |
| `category` | set when the queue prints one category |
| `is_internet` | set when the queue is only for internet registrations, or only for ones that are not |
| `second_name` | name shown to the operator |
| `print_cert` | this queue prints the certificate layout, category id plus 1000 |
| `auto_print` | the registration flow may enqueue without an extra confirm |
| `last_print` | milliseconds of the last successful pick. Memory only. A restart leaves every queue at 0 |

`PrinterRouting.toMap` drops `print_cert`, `auto_print`, and `no_postscript` when `second_name` is empty, because the last `getOrElse` returns an earlier map. The file always contains the whole record.

A pick reads the in-memory printers for the open event. It considers a queue only when `connected` and `installed` are both true.

1. An explicit queue name returns that queue.
2. Otherwise the candidates are queues with `used`. A queue with a category stays only when the category equals the visitor's. A queue with no category stays only when no used queue has that category. The internet flag works the same way: a set flag must equal the visitor's, and an empty flag stays only when no used queue has that flag.
3. When the caller passes a zone, a candidate whose zone is set and different is dropped. Among what remains, a queue with a zone is preferred to a queue with none. The oldest `last_print` wins a tie.
4. When the caller passes no zone, the oldest `last_print` among the candidates wins.

An empty candidate list is an error on the print response. The Scala client then calls `getAnyPrinter` and prints on whichever connected queue comes first, including a queue with `used` false. A successful pick stores `last_print` on that queue.

`print_cert` selects the layout after the queue is chosen. The layout id is the visitor category plus 1000. A missing certificate layout uses category 999, then category `-1`, then the smallest non-negative loaded category. `auto_print` is for the caller that enqueues. The pick does not read it.

### Where the code lives

```text
modules/ticket-render/          crate: layout JSON, draw, PNG, 1-bit graphic
src/registration_server/
  print_queue.rs                one worker, yield between pages
  cups.rs                       IPP search, one-time install, Print-Job
```

`POST /print` on the skeleton stays the enqueue. The body gains the registration id. The worker loads that row from the open event, picks the queue, and runs the job above. Sync stores `badge.cfg` and `bg.png` from `boxapi/getbadge` and does not call the crate.

Tests for ticket-render use fixture cfg files and compare PNG or graphic bytes. They do not start cupsd. Tests for `cups.rs` answer a local IPP stand-in on `127.0.0.1`. They do not call a real printer. A test that needs an event file uses a temp directory.

## What step 8 leaves

- The operator screens that edit routing and Zebra settings. The records and the save path exist. The React pages stay with the admin plan.
- Downloading CUPS filters onto the device.
- Windows / `winprint`.
- Godex.
- Changing `boxapi/getbadge`. The octet-stream and the Java cfg stay the wire format. The JSON layout is a local cache.
