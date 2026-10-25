# IPP

What a printer that speaks IPP reports, and how a caller reads it, changes it, and finds the printer. The cross-platform caller is [ipp-tool.md](ipp-tool.md). The operating-system queue list is [native-print.md](native-print.md). Step 12 of [printing-implementation-plan.md](printing-implementation-plan.md) runs both and joins them on the printer's address.

IPP is the job protocol. `Get-Printer-Attributes` reads the printer. `Set-Printer-Attributes` writes a setting the printer allows. `Print-Job` sends one document. `Get-Jobs` and `Get-Job-Attributes` read the queue. `Cancel-Job` stops a job. `Hold-Job` pauses one, and `Release-Job` lets it continue.

A request names `printer-uri`. An `ipp://` URI is IPP on TCP, port 631 unless the URI says otherwise. An `ipps://` URI is the same protocol over TLS.

## Document formats

`document-format-supported` is the list of MIME types this printer accepts. `document-format-default` is the type it assumes when a job names none. `document-format-preferred` is the type an IPP Everywhere printer asks for. A `Print-Job` sets `document-format` to one entry from the supported list.

| MIME type | What the bytes are |
|---|---|
| `application/pdf` | PDF. Common on a printer with a PDF RIP, and on IPP Everywhere. |
| `application/postscript` | PostScript, LanguageLevel 2 or 3. A PostScript-only printer accepts this and often does not accept PDF. |
| `application/octet-stream` | Raw bytes the printer auto-senses. ZPL and other vendor languages travel as this when the printer has no MIME of its own. |
| `image/png` | PNG. Usual as input to a CUPS queue. Rare as a type the printer itself advertises. |
| `image/jpeg` | JPEG. |
| `image/pwg-raster` | PWG Raster. Required for IPP Everywhere. |
| `image/urf` | Apple Raster, used by AirPrint. |
| `image/tiff` | TIFF. |
| `text/plain` | Plain text, one font, no layout. |
| `application/vnd.hp-pcl` | PCL. |

CUPS uses private types inside its filter graph: `application/vnd.cups-postscript`, `application/vnd.cups-pdf`, `application/vnd.cups-raster`, and `application/vnd.cups-raw`. Those name a filter stage. A printer does not advertise them. Asking cupsd for `document-format-supported` returns the types that queue's filter accepts. Asking the printer's own URI returns the types the printer consumes.

A PostScript printer advertises `application/postscript`. When the same RIP also reads PDF, `application/pdf` is on the list too. It does not advertise `image/png`. A PNG sent at that printer is converted by a driver before it reaches the device. A printer that already lists `application/pdf` or `image/pwg-raster` needs no driver for a job in that type.

## Settings

`Get-Printer-Attributes` returns the configuration. An attribute ending in `-supported` is the legal set. `-default` is the current choice. `-ready` is what is loaded now.

| Attribute | Meaning |
|---|---|
| `printer-state` | `idle`, `processing`, or `stopped`. |
| `printer-state-reasons` | Keywords such as `media-empty`, `media-low`, `toner-low`, `toner-empty`, `marker-supply-low`, `marker-supply-empty`, `cover-open`, `door-open`. |
| `printer-make-and-model` | The device string. |
| `media-supported`, `media-default`, `media-ready` | Sizes the printer can use, the size it will use, and the size that is loaded. A loaded tray is `media-col-ready`. |
| `media-type-supported` | Stock, such as `stationery`, `labels`, or `envelope`. |
| `sides-supported`, `sides-default` | `one-sided`, `two-sided-long-edge`, `two-sided-short-edge`. This is duplex. |
| `printer-resolution-supported`, `printer-resolution-default` | Resolutions, in dots per inch. |
| `print-color-mode-supported` | `auto`, `color`, `monochrome`, and the printer's other modes. |
| `operations-supported` | The IPP operations this printer implements. |
| `queued-job-count` | Jobs still in the printer. |

## Writing a setting

`printer-settable-attributes-supported` is the list `Set-Printer-Attributes` may change. A name absent from that list is read-only. A value absent from the matching `-supported` list is rejected. The write sends one attribute and reads the status back. A printer that omits `printer-settable-attributes-supported` accepts no setting from this client.

The settings this client writes are:

| Setting | IPP attribute | Values |
|---|---|---|
| Media size | `media` | One keyword from `media-supported`, such as `iso_a4_210x297mm` or `na_letter_8.5x11in`. |
| Media type | `media-type`, or `media-type` inside `media-col` | One keyword from `media-type-supported`. |
| Duplex | `sides` | `one-sided`, `two-sided-long-edge`, `two-sided-short-edge`. |
| Resolution | `printer-resolution` | One value from `printer-resolution-supported`. |
| Color | `print-color-mode` | One value from `print-color-mode-supported`. |

A refused write leaves the previous value. The client does not store a local copy and report success.

## Toner and ink

The supply attributes are the `marker-*` group. `marker-names` and `marker-types` name each supply (`toner`, `ink`, `ink-cartridge`). `marker-colors` is the color of each. `marker-levels` is the current amount, 0 through 100. `marker-low-levels` and `marker-high-levels` are the thresholds. A level of `-1` means unknown, `-2` means not empty, and `-3` means some remains but there is no percentage.

`printer-state-reasons` carries the coarse form: `toner-low`, `toner-empty`, `marker-supply-low`, `marker-supply-empty`. Those reasons appear on printers that omit `marker-levels`. A printer that returns neither has no toner reading.

## Discovery

IPP and IPPS printers are found with DNS-SD. The service types are `_ipp._tcp` and `_ipps._tcp`. The first becomes an `ipp://` URI. The second becomes an `ipps://` URI. The default port is 631. `_printer._tcp` is LPD and is not an IPP printer.

The TXT record supplies the rest of the URI and a label:

| TXT key | Use |
|---|---|
| `rp` | Resource path. Absent means `ipp/print`. The URI is `ipp://host:port/` plus that path. |
| `ty` | Make and model, shown when `printer-make-and-model` has not been read yet. |
| `UUID` | Stable identity when the address changes. |
| `note` | The operator's label, when the printer set one. |
| `qtotal` | Queued jobs at the moment of the browse. |

A browse returns a host name. The client resolves it to an address and keeps both. Two browse results with the same address and port are one printer. `ipps://` wins over `ipp://` for that pair when both answer.

A caller may also pass an `ipp://` or `ipps://` URI and skip the browse. That printer is queried with `Get-Printer-Attributes` the same way.

Refresh browses again, then sends `Get-Printer-Attributes` to each known printer. A printer that answers stays, with new formats, settings, and toner. A printer that does not answer leaves the available set. A URI added by hand is queried the same way and leaves the set when it does not answer.

## Joining an operating-system queue

A native queue whose device URI is `ipp://` or `ipps://` already names a host. The host is resolved to an address. That address is the same printer as a browse result with that address. The pair is one record. The native queue keeps its name. Formats, settings, toner, a test page, and a file in a supported type are read and sent through the IPP tool, once. The address is not queried a second time.

A USB queue, a `socket://` queue, and any other non-IPP queue stay native. They have no IPP format list and no IPP toner reading.
