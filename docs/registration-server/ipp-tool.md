# IPP tool

The cross-platform IPP client. It discovers IPP and IPPS printers, reads formats, settings, and toner, writes a setting the printer allows, prints a test page or a supported file, and lists and stops jobs. It does not link CUPS and it does not call the Windows print spooler. The attribute names and the discovery records are [ipp.md](ipp.md). The operating-system queue list that is joined with this tool is [native-print.md](native-print.md).

The library is `modules/ipp-tool`. It is standalone: Rust links the `rlib`, and C and Python load the same calls from the shared library through `include/ipp_tool.h`. The build sequence is [ipp-tool-implementation-plan.md](ipp-tool-implementation-plan.md). Tests stand in for DNS-SD and for the printer: a browse returns a fixed set of records, and an HTTP listener on `127.0.0.1` speaks IPP. Tests do not open a LAN socket and do not require a printer.

## Records

One discovered printer is `name`, `uri`, `address`, `port`, `secure`, `make_model`, `formats`, `settings`, `toner`, and `jobs`. `uri` is `ipp://` or `ipps://`. `secure` is true for IPPS. `formats` is `document-format-supported`, plus `document-format-default` and `document-format-preferred` when the printer sent them. `settings` is the supported, default, and ready values from [ipp.md](ipp.md). `toner` is the marker list, or the toner keywords from `printer-state-reasons` when levels are absent.

## Discover, refresh, add

`discover` browses `_ipp._tcp` and `_ipps._tcp`, resolves each host, and sends `Get-Printer-Attributes`. The returned set is the printers that answered.

`refresh` browses again and queries every printer already known, including one added by URI. A printer that answers is replaced with the new attributes. A printer that does not answer is dropped.

`add(uri)` takes one `ipp://` or `ipps://` URI, queries it, and inserts it. No browse runs. A URI that does not answer is an error and adds nothing.

Two results with the same address and port are one printer. When one of them is IPPS, that URI is the one used for later calls.

## Read

`info(uri)` sends `Get-Printer-Attributes` and fills formats, settings, and toner. The request asks for `document-format-supported`, `document-format-default`, `document-format-preferred`, the settings attributes, the marker attributes, `printer-state`, `printer-state-reasons`, and `queued-job-count`.

## Write a setting

`set(uri, setting, value)` checks `printer-settable-attributes-supported` and the matching `-supported` list, then sends `Set-Printer-Attributes`. The settings are media size (`media`), media type (`media-type`), duplex (`sides`), resolution (`printer-resolution`), and color (`print-color-mode`). A name the printer did not mark settable, or a value outside the supported list, returns an error and sends nothing. A rejected write returns the IPP status and leaves the previous value.

## Print

`print_test(uri)` builds a one-page page and sends `Print-Job`. The format is `document-format-preferred` when that value is also supported. Otherwise it is the first supported type in this order: `application/pdf`, `application/postscript`, `image/pwg-raster`, `image/jpeg`, `image/png`. A printer that supports none of those returns an error and sends no job. The call returns the job id.

`print_file(uri, path)` maps the file to a MIME type and sends it only when that type is in `document-format-supported`.

| Extension | MIME type |
|---|---|
| `.pdf` | `application/pdf` |
| `.ps`, `.eps` | `application/postscript` |
| `.png` | `image/png` |
| `.jpg`, `.jpeg` | `image/jpeg` |
| `.pwg` | `image/pwg-raster` |
| `.urf` | `image/urf` |
| `.tif`, `.tiff` | `image/tiff` |
| `.txt` | `text/plain` |
| `.pcl` | `application/vnd.hp-pcl` |

A type the printer does not list is an error. No `Print-Job` is sent. `application/octet-stream` is used only when the caller passes the file as raw. The call returns the job id.

## Jobs

`jobs(uri)` sends `Get-Jobs` for jobs that are not completed, then the completed ones, newest first. Each entry is `job-id`, `job-state`, `job-state-reasons`, `job-name`, and `job-impressions-completed`.

`job(uri, id)` sends `Get-Job-Attributes` for one id.

`cancel(uri, id)` sends `Cancel-Job`. That is how a job is interrupted. The job leaves the processing state. `hold(uri, id)` sends `Hold-Job` and leaves the job in the queue. `release(uri, id)` sends `Release-Job`. A cancel of an unknown id returns the printer's status and does not invent a local job.
