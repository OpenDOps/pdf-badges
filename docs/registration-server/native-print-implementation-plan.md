# Native print

Build sequence for [native-print.md](native-print.md). Linux and macOS read CUPS. Windows reads the spooler. Direct IPP discovery, formats, settings, toner, and a file sent to a printer URI are [ipp-tool.md](ipp-tool.md) and are not this library. Step 12 of [printing-implementation-plan.md](printing-implementation-plan.md) calls `list` after this plan is done.

The library is `registration_server::native_print`. It returns queues the operating system already has. It does not browse `_ipp._tcp`. It does not send `Print-Job` to a printer URI. It does not download a filter and does not rewrite a PPD.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

The library is done when steps 1 through 8 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Record](#step-1-record) | The queue record, kind, dpi, and width | done |
| [2. Host](#step-2-host) | An address from an `ipp://` or `ipps://` URI | done |
| [3. CUPS](#step-3-cups) | `CUPS-Get-Printers` on Linux and macOS | done |
| [4. Choices](#step-4-choices) | PPD options except `PageSize` | done |
| [5. Spooler](#step-5-spooler) | A Windows queue becomes the same record | done |
| [6. Port](#step-6-port) | A Windows port becomes `device_uri` and `host` | done |
| [7. List](#step-7-list) | `list` calls CUPS or the spooler | done |
| [8. Embed](#step-8-embed) | C ABI and a Python wrapper | done |

Shared rules:

- Tests are `cargo test --lib registration_server::native_print`. The CUPS stand-in is HTTP on `127.0.0.1` and speaks IPP. Tests do not start cupsd.
- Linux and macOS compile steps 1–4 and the CUPS `list`. Windows compiles steps 5–7. A Windows build does not compile the CUPS client, and a Linux or macOS build does not compile the spooler client. The queue record is the same shape on both.
- Steps 5 and 6 are a pure mapping of name, driver, port, and status. The step 7 Windows test installs that mapping as a spooler fixture, so it does not call `EnumPrinters`. `list` calls `EnumPrinters` when no fixture is installed. `ipp-tool` is a dependency of `rust-reg` on every platform. The CUPS client that calls it is compiled only on Linux and macOS.
- `installed` is true because the queue exists. `connected` is left false for a non-IPP queue. Step 13 of the printing plan sets that flag from `CUPS-Get-Devices`. An `ipp://` or `ipps://` queue is connected when the IPP tool has attributes for that address, which step 12 of the printing plan does. This library does not browse.
- The C and Python callers use one list. They do not name CUPS or the spooler. The shared library is built for the target, so a Linux or macOS build contains the CUPS client and a Windows build contains the spooler client.

## Step 1. Record

[Back to summary](#summary)

One queue is one record. Kind and size come from the make and model string. No operating-system call runs.

### Work

1. `Queue` is `name`, `device_uri`, `make_model`, `state`, `accepting`, `host`, `kind`, `dpi`, `width_in`, `choices`, `installed`, and `connected`.
2. Make `zebra technologies` or `tsc`, compared case-insensitively, sets `kind` to `zpl`. Any other make sets `raster`.
3. Model `ztc gx430t` and make `evolis` set `dpi` 300. Other ZPL queues set `dpi` 203 and `width_in` 4.1. Evolis sets `width_in` 2.125. A raster queue leaves `width_in` empty.
4. `choices` starts empty. `host` starts empty. `installed` is true. `connected` is false.

### Test scenarios

| Scenario | Assert |
|---|---|
| `zebra_and_tsc_are_zpl` | Make `Zebra Technologies` and model `ZTC GX430t` is `zpl`, dpi 300, width 4.1. Make `TSC` is `zpl`, dpi 203, width 4.1. |
| `evolis_is_raster` | Make `Evolis` is `raster`, dpi 300, width 2.125. |
| `another_make_is_raster` | Make `HP` is `raster`, dpi 300, and `width_in` is empty. |

### Done when

`cargo test --lib registration_server::native_print::tests::record` passes.

## Step 2. Host

[Back to summary](#summary)

`host` is set only for an IPP device URI. USB and raw sockets stay empty, so step 12 of the printing plan does not join them to a discovered printer.

### Work

1. An `ipp://` or `ipps://` URI yields the host, the port, and the address when the host is a literal. A host name is stored and resolved by the caller that has a resolver. The test passes the resolved address.
2. `usb://`, `socket://`, `dnssd://`, and any other scheme leave `host` empty.
3. A missing host in an `ipp://` URI is an error and names the URI.

### Test scenarios

| Scenario | Assert |
|---|---|
| `an_ipp_uri_yields_the_address` | `ipp://192.0.2.10/ipp/print` sets `host` to `192.0.2.10` and the port to 631. `ipps://printer.local:443/ipp/print` keeps the name `printer.local` and the port 443. |
| `usb_and_socket_have_no_host` | `usb://Zebra/ZTC?serial=ABC` and `socket://192.0.2.10:9100` leave `host` empty. |

### Done when

`cargo test --lib registration_server::native_print::tests::host` passes, and step 1 stays green.

## Step 3. CUPS

[Back to summary](#summary)

On Linux and macOS, `CUPS-Get-Printers` becomes one record per queue.

### Work

1. Send `CUPS-Get-Printers` to the local cupsd. Read the queue name, device URI, `printer-make-and-model`, `printer-state`, and whether the queue accepts jobs.
2. `printer-state` `idle`, `processing`, and `stopped` are stored as those words. Accepting jobs is `accepting`.
3. Step 1 fills `kind`, `dpi`, and `width_in`. Step 2 fills `host`.
4. This step does not send `CUPS-Get-Devices`, `CUPS-Add-Modify-Printer`, or `Print-Job`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `queues_become_records` | The stand-in lists a Zebra `ztc gx430t` at `usb://Zebra/ZTC?serial=ABC`, a queue at `ipp://192.0.2.10/ipp/print`, and an Evolis. The Zebra record is `zpl`, dpi 300, `host` empty. The IPP record has `host` `192.0.2.10`. The Evolis record is `raster`, width 2.125. |
| `get_printers_is_the_only_call` | The stand-in sees one `CUPS-Get-Printers` and no `Print-Job`. |

### Done when

`cargo test --lib registration_server::native_print::tests::cups` passes, and steps 1 and 2 stay green.

## Step 4. Choices

[Back to summary](#summary)

`choices` are the PPD options the operator may set. `PageSize` is left out. Media size comes from the layout on the job.

### Work

1. Read the PPD option list for each CUPS queue. Each option is a name and the allowed values.
2. Drop `PageSize`. Every other option is stored on `choices`.
3. A queue with no PPD has an empty `choices`. The rest of the record stays.

### Test scenarios

| Scenario | Assert |
|---|---|
| `page_size_is_omitted` | The PPD lists `PageSize` and `Resolution`. `choices` has `Resolution` and does not have `PageSize`. |
| `a_missing_ppd_leaves_choices_empty` | The queue has no PPD. `choices` is empty. `kind` is unchanged. |

### Done when

`cargo test --lib registration_server::native_print::tests::choices` passes, and steps 1 through 3 stay green.

## Step 5. Spooler

[Back to summary](#summary)

A Windows queue becomes the same `Queue`. The mapping is pure. `EnumPrinters` is the next step's call.

### Work

1. `from_spooler(name, driver, port, status)` fills `name`, `make_model` from the driver string, `state`, and `accepting`.
2. `state` is `processing` when the status has printing or processing. It is `stopped` when the status has offline, not available, or error. Otherwise it is `idle`.
3. `accepting` is false when the status has paused, offline, or not available. Otherwise it is true.
4. Step 1 fills `kind`, `dpi`, and `width_in` from the driver string the same way it fills them from a CUPS make and model. `installed` is true. `connected` is false until step 6 sets `host` for an IPP port.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_zebra_driver_is_zpl` | Driver `ZTC GX430t` and a printing status is `zpl`, dpi 300, state `processing`, `accepting` true. |
| `an_offline_queue_is_stopped` | An offline status is `stopped` and `accepting` false. The name is the spooler name. |
| `an_hp_driver_is_raster` | Driver `HP LaserJet` is `raster`, dpi 300, and `width_in` is empty. |

### Done when

`cargo test --lib registration_server::native_print::tests::spooler` passes on Windows. That build does not compile steps 1 through 4.

## Step 6. Port

[Back to summary](#summary)

The Windows port string becomes `device_uri`. `host` is set only when that URI is `ipp://` or `ipps://`.

### Work

1. A port that is already `ipp://` or `ipps://` is `device_uri`. Step 2 sets `host`.
2. A port that is `https://` and whose path contains `ipp` becomes `ipps://` with the same host, port, and path. `http://` with that path becomes `ipp://`.
3. A port whose name begins with `USB` becomes `usb://` plus the driver and the port, with spaces in the driver turned into `_`. `host` stays empty.
4. A port named `IP_` plus an address, or `IP_` plus an address and a port, becomes `socket://` that address and port. The port defaults to 9100. `host` stays empty.
5. Any other port, including a WSD name, becomes `winspool://` plus that port. `host` stays empty.

### Test scenarios

| Scenario | Assert |
|---|---|
| `an_ipp_port_sets_the_host` | Port `ipp://192.0.2.10/ipp/print` sets `device_uri` to that string and `host` to `192.0.2.10`. |
| `https_ipp_becomes_ipps` | Port `https://192.0.2.11/ipp/print` becomes `ipps://192.0.2.11/ipp/print` and `host` is `192.0.2.11`. |
| `a_usb_port_has_no_host` | Port `USB001` and driver `Zebra Technologies` becomes `usb://Zebra_Technologies/USB001`. `host` is empty. |
| `a_tcp_port_is_a_socket` | Port `IP_192.0.2.12` becomes `socket://192.0.2.12:9100`. `host` is empty. |

### Done when

`cargo test --lib registration_server::native_print::tests::port` passes on Windows, and the spooler tests stay green.

## Step 7. List

[Back to summary](#summary)

`list` is the one function step 12 of the printing plan calls. Linux and macOS use step 3. Windows uses `EnumPrinters` and steps 5 and 6.

### Work

1. On Linux and macOS, `list` sends `CUPS-Get-Printers` and returns those records. It does not call `EnumPrinters`.
2. On Windows, `list` calls `EnumPrinters` for local queues and connections, then `from_spooler` and the port mapping for each. It does not send `CUPS-Get-Printers`.
3. The record fields are the same on both. A later caller does not grow a second map.
4. `list` does not browse `_ipp._tcp` and does not send `Print-Job`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `unix_list_is_cups` | On Linux and macOS the stand-in sees one `CUPS-Get-Printers`. The returned queues are the step 3 records. |
| `windows_list_is_the_spooler` | The spooler fixture has one IPP port and one USB port. `list` returns two records. The IPP record has `host` set. The USB record does not. No CUPS operation is sent. |

### Done when

`cargo test --lib registration_server::native_print::tests::list` passes on the system that compiles that half. Linux and macOS run `unix_list_is_cups` with steps 1 through 4. Windows runs `windows_list_is_the_spooler` with steps 5 and 6.

## Step 8. Embed

[Back to summary](#summary)

C and Python call the same list. The header does not name CUPS or the spooler. The shared library built for the machine contains that platform's client.

### Work

1. `include/native_print.h` is the contract. `native_print_list` returns the queues. The other calls read one field by index. `service` NULL uses the local print service. A non-NULL service is that service's address. Each call returns `0` on success and a non-zero code on failure. The message is thread-local, read with `native_print_last_error`. A panic at this boundary becomes that error and does not unwind into the caller.
2. The symbols are exported from the `rust-reg` shared library: `librust_reg.dylib` on macOS, `librust_reg.so` on Linux, and `rust_reg.dll` on Windows. `scripts/release/pack.sh` builds that library with `--lib` for the machine it is run on.
3. `modules/native-print/native_print.py` loads the shared library with ctypes and returns the same queue fields. `NATIVE_PRINT_LIB` overrides the library path. Otherwise the wrapper looks in `target/debug` and `target/release`.

### Test scenarios

The C cases are Rust tests that call the `extern "C"` functions the way a C program would. They do not shell out to a C compiler. The Python case runs `modules/native-print/tests/test_embed.py`.

| Scenario | Assert |
|---|---|
| `c_abi_lists_the_queues` | `native_print_list` returns `0`. The Zebra queue is `zpl`, dpi 300, width `4.1`, and `host` empty. The IPP queue has `host` `192.0.2.10`. A missing out-pointer returns non-zero and the error names `output`. |
| `python_embed_lists_the_queues` | The wrapper reads those same fields from the shared library. A service that does not answer raises, and the message is the text from `native_print_last_error`. |

### Done when

`cargo test --lib registration_server::native_print::abi` passes, and steps 1 through 7 stay green on the system that compiles them. `cargo build --lib` produces the shared library for this machine.
