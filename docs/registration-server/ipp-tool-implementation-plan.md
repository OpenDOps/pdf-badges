# IPP tool

Build sequence for [ipp-tool.md](ipp-tool.md). The attributes, the MIME types, and IPP/IPPS discovery are [ipp.md](ipp.md). The operating-system queue that is joined onto a discovered printer is [native-print.md](native-print.md). Step 12 of [printing-implementation-plan.md](printing-implementation-plan.md) calls `discover` after this plan is done.

The crate is `modules/ipp-tool`. It is a standalone library: Rust links the `rlib`, and C and Python load the same functions from the shared library. It does not link CUPS, it does not call the Windows spooler, and it does not depend on `rust-reg`. A browse is a trait. Tests pass records and do not open a LAN socket. The printer is an HTTP listener on `127.0.0.1` that speaks IPP. `ipps://` in a test is the same listener with the secure flag set. Tests do not require a printer.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

The tool is done when steps 1 through 14 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Record](#step-1-record) | The crate and one printer record | done |
| [2. Message](#step-2-message) | Encode and decode the IPP operations this tool sends | done |
| [3. Attributes](#step-3-attributes) | `Get-Printer-Attributes` over HTTP | done |
| [4. Formats](#step-4-formats) | `document-format-supported` and the two companions | done |
| [5. Settings](#step-5-settings) | Supported, default, and ready values | done |
| [6. Toner](#step-6-toner) | Marker levels, or the toner reasons when levels are absent | done |
| [7. Set](#step-7-set) | `Set-Printer-Attributes` for a settable value | done |
| [8. Test page](#step-8-test-page) | One page in the preferred supported type | not started |
| [9. File](#step-9-file) | A file whose MIME type the printer lists | not started |
| [10. Jobs](#step-10-jobs) | List, one job, cancel, hold, release | not started |
| [11. Address](#step-11-address) | One printer per address and port. IPPS wins | not started |
| [12. Discover](#step-12-discover) | `_ipp._tcp` and `_ipps._tcp`, then the attributes | not started |
| [13. Refresh](#step-13-refresh) | Browse again, query known printers, add a URI | not started |
| [14. Embed](#step-14-embed) | C ABI and a Python wrapper over the same calls | done |

Step 14 is done ahead of steps 8 through 13. Those steps stay not started. The header calls the Rust client for print, jobs, cancel, hold, release, discover, refresh, and add.

Shared rules:

- Tests are `cargo test -p ipp-tool`. They do not open a LAN socket and do not start cupsd.
- A request names `printer-uri`. `ipp://` is HTTP POST with `Content-Type: application/ipp`, port 631 unless the URI says otherwise. `ipps://` is the same body over TLS.
- The operations are `Get-Printer-Attributes`, `Set-Printer-Attributes`, `Print-Job`, `Get-Jobs`, `Get-Job-Attributes`, `Cancel-Job`, `Hold-Job`, and `Release-Job`.
- A refused IPP status is returned to the caller. The tool does not store a local copy and report success.
- The package builds `rlib`, `cdylib`, and `staticlib` from this crate. Symbol names start with `ipp_tool_`. The header is `include/ipp_tool.h`. Python loads the shared library with ctypes from `modules/ipp-tool/ipp_tool.py`. The registration server is one Rust caller of the `rlib`. C and Python call the header. They do not grow a second IPP client.

## Step 1. Record

[Back to summary](#summary)

The crate exists, and one discovered printer is a struct. No socket is opened.

### Work

1. Add `modules/ipp-tool` and the path dependency from the workspace. The crate does not depend on CUPS or on `rust-reg`.
2. `Printer` is `name`, `uri`, `address`, `port`, `secure`, `make_model`, `formats`, `settings`, `toner`, and `jobs`. `secure` is true when the URI is `ipps://`.
3. `formats` holds `document-format-supported`, and `document-format-default` and `document-format-preferred` when present. `settings` and `toner` and `jobs` start empty.

### Test scenarios

| Scenario | Assert |
|---|---|
| `an_ipps_uri_is_secure` | `ipps://192.0.2.10/ipp/print` has `secure` true, port 631, and address `192.0.2.10`. |
| `an_ipp_uri_is_not_secure` | `ipp://192.0.2.10:631/ipp/print` has `secure` false. |

### Done when

`cargo test -p ipp-tool -- record` passes.

Done. `Printer::from_uri` fills `secure`, `port`, and `address`. `formats`, `settings`, `toner`, and `jobs` start empty.

## Step 2. Message

[Back to summary](#summary)

The tool encodes a request and decodes a response. This step does not open a socket.

### Work

1. Encode an IPP 2.0 request: version, operation id, request id, `attributes-charset` `utf-8`, `attributes-natural-language` `en`, the operation attributes, and the printer attributes when a write sends them.
2. Decode a response: status code, and the groups the later steps read. An unknown attribute is skipped. A truncated message is an error.
3. Operation ids match the eight operations in the shared rules.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_request_roundtrips` | Encoding `Get-Printer-Attributes` for `ipp://192.0.2.10/ipp/print` and decoding that body yields the same operation and `printer-uri`. |
| `a_truncated_body_is_an_error` | A body shorter than the header is an error and names the length. |

### Done when

`cargo test -p ipp-tool -- message` passes, and step 1 stays green.

Done. An IPP 2.0 request encodes and decodes. A body shorter than the header is `truncated IPP message, length N`.

## Step 3. Attributes

[Back to summary](#summary)

`info` sends `Get-Printer-Attributes` and fills the record from the response.

### Work

1. POST the encoded request to the URI. The stand-in is `http://127.0.0.1` and a port the test chose.
2. The request asks for `document-format-supported`, `document-format-default`, `document-format-preferred`, `printer-settable-attributes-supported`, the settings attributes, the marker attributes, `printer-state`, `printer-state-reasons`, `printer-make-and-model`, and `queued-job-count`.
3. A connect failure, a timeout, or a body that is not IPP is an error. The record is left unchanged.

### Test scenarios

| Scenario | Assert |
|---|---|
| `info_reads_the_make` | The stand-in returns `printer-make-and-model` `Zebra`. `make_model` is `Zebra`. The request URI is the one passed in. |
| `a_down_printer_is_an_error` | The stand-in accepts nothing. `info` returns an error and the previous `make_model` stays. |

### Done when

`cargo test -p ipp-tool -- attributes` passes, and steps 1 and 2 stay green.

Done. `Printer::info` posts `Get-Printer-Attributes` to the URI. The stand-in is HTTP on `127.0.0.1`. A connect failure or a body that is not IPP leaves `make_model` as it was.

## Step 4. Formats

[Back to summary](#summary)

The format list is the MIME types the printer accepts.

### Work

1. `document-format-supported` becomes `formats.supported`. The values include `application/pdf`, `application/postscript`, `application/octet-stream`, `image/png`, `image/jpeg`, `image/pwg-raster`, `image/urf`, `image/tiff`, `text/plain`, and `application/vnd.hp-pcl` when the printer sends them.
2. `document-format-default` and `document-format-preferred` are stored when present and absent when omitted.
3. A CUPS private type such as `application/vnd.cups-postscript` is kept if the printer sent it, and it is not invented when the list omits it.

### Test scenarios

| Scenario | Assert |
|---|---|
| `supported_formats_are_kept` | The stand-in lists `application/pdf`, `application/postscript`, and `image/pwg-raster`. Those three are `formats.supported`, in that order. |
| `preferred_is_optional` | A response with no `document-format-preferred` leaves that field empty. `application/pdf` stays supported. |

### Done when

`cargo test -p ipp-tool -- formats` passes, and steps 1 through 3 stay green.

Done. `formats.supported` is the MIME list in the order the printer sent it, including a CUPS private type. An omitted preferred value stays empty.

## Step 5. Settings

[Back to summary](#summary)

Supported, default, and ready values are stored for the settings [ipp.md](ipp.md) names.

### Work

1. Store `printer-state`, `printer-state-reasons`, `media-supported`, `media-default`, `media-ready`, `media-col-ready`, `media-type-supported`, `sides-supported`, `sides-default`, `printer-resolution-supported`, `printer-resolution-default`, `print-color-mode-supported`, `operations-supported`, and `queued-job-count`.
2. An omitted attribute stays empty. A present attribute replaces the previous value on the next `info`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `sides_and_media_are_read` | `sides-supported` is `one-sided` and `two-sided-long-edge`. `sides-default` is `one-sided`. `media-default` is `iso_a4_210x297mm`. |
| `an_omitted_resolution_stays_empty` | The response has no `printer-resolution-supported`. That list is empty. `sides-default` is still `one-sided`. |

### Done when

`cargo test -p ipp-tool -- settings` passes, and steps 1 through 4 stay green.

Done. A present attribute replaces the stored value. An omitted attribute is empty after `info`.

## Step 6. Toner

[Back to summary](#summary)

Marker levels are the toner reading. The reasons stand in when the printer sends no levels.

### Work

1. Store `marker-names`, `marker-types`, `marker-colors`, `marker-levels`, `marker-low-levels`, and `marker-high-levels` as parallel lists.
2. A level of `-1`, `-2`, or `-3` is kept as that number.
3. When `marker-levels` is absent, toner is the keywords `toner-low`, `toner-empty`, `marker-supply-low`, and `marker-supply-empty` found in `printer-state-reasons`. When neither is present, toner is empty.

### Test scenarios

| Scenario | Assert |
|---|---|
| `levels_are_the_reading` | Names `black` and `cyan`, types `toner`, levels `40` and `-1`. The second level stays `-1`. |
| `reasons_stand_in_for_levels` | No marker attributes. `printer-state-reasons` contains `toner-low`. Toner is that keyword. |
| `neither_is_an_empty_reading` | No marker attributes and no toner reason. Toner is empty. `printer-state` is still stored. |

### Done when

`cargo test -p ipp-tool -- toner` passes, and steps 1 through 5 stay green.

Done. Marker levels are the reading, including `-1`. When levels are absent, toner is the toner keywords from `printer-state-reasons`.

## Step 7. Set

[Back to summary](#summary)

A setting is written only when the printer lists it as settable and the value is supported.

### Work

1. `set(uri, setting, value)` accepts media size (`media`), media type (`media-type`), duplex (`sides`), resolution (`printer-resolution`), and color (`print-color-mode`).
2. The name must be in `printer-settable-attributes-supported`, and the value must be in the matching `-supported` list. Otherwise nothing is sent.
3. A printer that omits `printer-settable-attributes-supported` accepts no setting.
4. `Set-Printer-Attributes` sends that one attribute. A success status replaces the stored default. A rejected status is returned and the stored default stays.

### Test scenarios

| Scenario | Assert |
|---|---|
| `duplex_is_written` | `sides` is settable and `two-sided-long-edge` is supported. The stand-in sees `Set-Printer-Attributes` with that value. The stored default becomes `two-sided-long-edge`. |
| `a_readonly_name_sends_nothing` | `sides` is absent from the settable list. The stand-in sees no write. The error names `sides`. |
| `a_rejected_write_keeps_the_default` | The stand-in returns a client-error status. `sides-default` stays `one-sided`. |

### Done when

`cargo test -p ipp-tool -- set` passes, and steps 1 through 6 stay green.

Done. `Printer::set` sends `Set-Printer-Attributes` only when the name is settable and the value is supported. A rejected status leaves the stored default.

## Step 8. Test page

[Back to summary](#summary)

`print_test` sends one page. The format is the printer's preferred type when that type is supported, otherwise the first supported type in a fixed order.

### Work

1. The order is `application/pdf`, `application/postscript`, `image/pwg-raster`, `image/jpeg`, `image/png`. `document-format-preferred` wins when it is also in `document-format-supported`.
2. A printer that supports none of those five returns an error. No `Print-Job` is sent.
3. The body is one page in the chosen type. PDF begins with `%PDF`. PostScript begins with `%!PS`. PNG and JPEG begin with their file signatures. PWG Raster begins with `RaS2`.
4. The call returns `job-id` from the response.

### Test scenarios

| Scenario | Assert |
|---|---|
| `preferred_pdf_is_sent` | Preferred and supported are `application/pdf`. The stand-in sees `document-format` `application/pdf` and a body that begins with `%PDF`. The returned id is the one in the response. |
| `the_order_picks_postscript` | Supported types are `image/jpeg` and `application/postscript`, and preferred is absent. The body begins with `%!PS`. |
| `no_usable_type_sends_nothing` | Supported is only `text/plain`. The stand-in sees no `Print-Job`. |

### Done when

`cargo test -p ipp-tool -- test_page` passes, and steps 1 through 7 stay green.

## Step 9. File

[Back to summary](#summary)

`print_file` sends the file bytes when the mapped type is in `document-format-supported`.

### Work

1. The extension map is the table in [ipp-tool.md](ipp-tool.md): `.pdf`, `.ps`, `.eps`, `.png`, `.jpg`, `.jpeg`, `.pwg`, `.urf`, `.tif`, `.tiff`, `.txt`, `.pcl`.
2. A mapped type absent from `document-format-supported` is an error. No `Print-Job` is sent.
3. `application/octet-stream` is used only when the caller marks the file raw. An unmarked file with no mapped extension is an error.
4. A sent job returns `job-id`. The body is the file bytes.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_pdf_file_is_sent` | `page.pdf` and supported `application/pdf`. The stand-in sees that document format and the file bytes. |
| `a_png_the_printer_rejected_is_not_sent` | `page.png` and supported `application/pdf` only. The stand-in sees no `Print-Job`. |
| `raw_is_octet_stream` | The caller marks the file raw. The document format is `application/octet-stream` and the printer lists that type. |

### Done when

`cargo test -p ipp-tool -- file` passes, and steps 1 through 8 stay green.

## Step 10. Jobs

[Back to summary](#summary)

The queue can be listed, and one job can be cancelled, held, or released.

### Work

1. `jobs` sends `Get-Jobs` for jobs that are not completed, then for completed jobs. Each entry is `job-id`, `job-state`, `job-state-reasons`, `job-name`, and `job-impressions-completed`. Newest first.
2. `job` sends `Get-Job-Attributes` for one id.
3. `cancel` sends `Cancel-Job`. That is how a job is interrupted. `hold` sends `Hold-Job`. `release` sends `Release-Job`.
4. An unknown id returns the printer's status. The tool does not invent a local job.

### Test scenarios

| Scenario | Assert |
|---|---|
| `jobs_list_the_open_ones_first` | One processing job and one completed job. The processing job is first. |
| `cancel_interrupts_the_job` | `cancel` of id 7 sends `Cancel-Job` for id 7. The stand-in then reports that job `canceled`. |
| `an_unknown_id_is_the_printer_status` | `cancel` of id 9 returns the stand-in's client-error status. The job list is unchanged. |

### Done when

`cargo test -p ipp-tool -- jobs` passes, and steps 1 through 9 stay green.

## Step 11. Address

[Back to summary](#summary)

Two results with the same address and port are one printer. IPPS is the URI kept for later calls.

### Work

1. Resolve a host name to an address. Keep the name and the address. A literal address is stored as itself.
2. A second result with the same address and port replaces the first. When either URI is `ipps://`, the kept URI is `ipps://`.
3. Different ports on the same address stay two printers.

### Test scenarios

| Scenario | Assert |
|---|---|
| `ipps_replaces_ipp` | `ipp://192.0.2.10:631/ipp/print` and `ipps://192.0.2.10:631/ipp/print` are one printer, and the URI is the `ipps` one. |
| `two_ports_stay_two_printers` | Port 631 and port 443 on `192.0.2.10` are two records. |

### Done when

`cargo test -p ipp-tool -- address` passes, and steps 1 through 10 stay green.

## Step 12. Discover

[Back to summary](#summary)

`discover` browses IPP and IPPS, then reads attributes for each result that answers.

### Work

1. The browse trait returns service type, host, port, and TXT. The production browser queries `_ipp._tcp` and `_ipps._tcp`. Tests pass the records. `_printer._tcp` is ignored.
2. TXT `rp` is the resource path. Absent means `ipp/print`. `ty` is the make and model until `printer-make-and-model` arrives. The URI is `ipp://` or `ipps://` plus host, port, and that path. The default port is 631.
3. Each URI is passed to `info`. A result that does not answer is left out. Two results that step 11 collapses are queried once, at the kept URI.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_browse_becomes_a_printer` | TXT `rp` is `ipp/print` and the host resolves to `192.0.2.10`. The record URI is `ipp://192.0.2.10:631/ipp/print` and `make_model` is the attribute value, not `ty`. |
| `a_silent_browse_result_is_omitted` | One result answers and one does not. The set has the one that answered. |
| `lpd_is_ignored` | A `_printer._tcp` record produces no URI and no `Get-Printer-Attributes`. |

### Done when

`cargo test -p ipp-tool -- discover` passes, and steps 1 through 11 stay green.

## Step 13. Refresh

[Back to summary](#summary)

`refresh` browses again and queries printers already known. `add` inserts a URI without a browse.

### Work

1. `refresh` runs the step 12 browse, then `info` on every known printer, including one inserted by `add`. A printer that answers is replaced. A printer that does not answer is dropped.
2. `add(uri)` accepts one `ipp://` or `ipps://` URI, calls `info`, and inserts it. No browse runs. A URI that does not answer is an error and adds nothing.
3. This step does not call CUPS and does not call the Windows spooler.

### Test scenarios

| Scenario | Assert |
|---|---|
| `refresh_drops_a_silent_printer` | A known printer does not answer. It leaves the set. One that answers keeps the new `queued-job-count`. |
| `add_takes_a_uri` | `add("ipps://192.0.2.11/ipp/print")` inserts that printer. The browse trait is not called. |
| `a_dead_uri_adds_nothing` | `add` of a URI that does not answer returns an error. The set length stays the same. |

### Done when

`cargo test -p ipp-tool -- refresh` passes, and steps 1 through 12 stay green.

## Step 14. Embed

[Back to summary](#summary)

`info` and `set` are what other programs link today. Rust calls them on `ipp_tool`. C calls the same functions through a C ABI. Python loads the shared library and calls the header. This is the same shape as `pdf_template` in [template/implementation-plan.md](../pdf-tooling/template/implementation-plan.md) step 8: one `rlib`, one `cdylib`, one `staticlib`, one header, and a ctypes wrapper. Steps 8 through 13 fill in the calls this step already names.

### Work

1. `info`, `set`, `print_test`, `print_file`, `jobs`, `cancel`, `hold`, `release`, `discover`, `refresh`, and `add` call the Rust client. A Rust program depends on `ipp-tool` and calls the Rust functions directly. Steps 8 through 13 stay not started.
2. `[lib] crate-type` is `rlib`, `cdylib`, and `staticlib`. The shared library is `libipp_tool.dylib` on macOS, `libipp_tool.so` on Linux, and `ipp_tool.dll` on Windows. Symbol names start with `ipp_tool_`, so a later rename of the file does not rename the calls.
3. `modules/ipp-tool/src/ffi.rs` and `include/ipp_tool.h`. The header is the contract. Each call returns `0` on success and a non-zero code on failure. The message is thread-local, read with `ipp_tool_last_error`. A panic at this boundary becomes that error and does not unwind into the caller.

```c
int ipp_tool_printer_open(const char *uri, IppPrinter **out);
int ipp_tool_info(IppPrinter *printer);
const char *ipp_tool_make_model(const IppPrinter *printer);
int ipp_tool_set(IppPrinter *printer, const char *setting, const char *value);
int ipp_tool_print_test(IppPrinter *printer, int *job_id);
int ipp_tool_print_file(IppPrinter *printer, const char *path, int raw, int *job_id);
int ipp_tool_jobs(IppPrinter *printer, IppJob **out, size_t *count);
int ipp_tool_cancel(IppPrinter *printer, int job_id);
int ipp_tool_hold(IppPrinter *printer, int job_id);
int ipp_tool_release(IppPrinter *printer, int job_id);
int ipp_tool_discover(IppPrinter ***out, size_t *count);
int ipp_tool_refresh(IppPrinter ***printers, size_t *count);
int ipp_tool_add(const char *uri, IppPrinter **out);
void ipp_tool_printer_free(IppPrinter *printer);
void ipp_tool_list_free(IppPrinter **list, size_t count);
const char *ipp_tool_last_error(void);
```

   `IppPrinter` is an opaque pointer. Formats, settings, and toner are read from that pointer after `ipp_tool_info`. `ipp_tool_set` is step 7: a name that is not settable, or a value outside the supported list, returns non-zero and sends nothing. A refused IPP status returns non-zero and leaves the stored default. On failure an out-pointer is not set. The caller frees a printer with `ipp_tool_printer_free` and a list with `ipp_tool_list_free`, each once.

4. A thin Python wrapper embeds that shared library. It lives at `modules/ipp-tool/ipp_tool.py` and uses ctypes against the header. `Printer` opens a URI, reads the make and model, sets a setting, prints, and lists jobs. `IPP_TOOL_LIB` overrides the library path. Otherwise the wrapper looks in `target/debug` and `target/release`.

### Test scenarios

The C cases are Rust tests that call the `extern "C"` functions the way a C program would. They do not shell out to a C compiler. The Python case runs `modules/ipp-tool/tests/test_embed.py`, which loads the shared library in process. The stand-in is still HTTP on `127.0.0.1`.

| Scenario | Assert |
|---|---|
| `c_abi_info_reads_the_make` | `ipp_tool_printer_open` on the stand-in URI, then `ipp_tool_info`. `ipp_tool_make_model` is `Zebra`. The call returns `0`. |
| `c_abi_readonly_set_sends_nothing` | `sides` is absent from the settable list. `ipp_tool_set` returns non-zero. `ipp_tool_last_error` contains `sides`. The stand-in sees no `Set-Printer-Attributes`. |
| `python_embed_reads_the_make` | The wrapper opens the stand-in URI and reads make and model `Zebra`. A URI that does not answer raises, and the message is the text from `ipp_tool_last_error`. |
| `c_abi_print_test_sends_pdf` | `ipp_tool_info`, then `ipp_tool_print_test`. The stand-in sees a body that begins with `%PDF` and the returned id is 4. |

### Done when

`cargo test -p ipp-tool` passes, including the C ABI cases and `python_embed_reads_the_make`. `cargo build -p ipp-tool` produces the shared library and the static library. Steps 1 through 7 stay green. Steps 8 through 13 stay not started.

Done. `include/ipp_tool.h` and `modules/ipp-tool/ipp_tool.py` call the Rust client. Steps 8 through 13 stay not started.
