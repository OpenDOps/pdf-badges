# Native print

The operating-system printer list. Linux and macOS read CUPS. Windows reads the Windows print spooler. Direct IPP discovery, formats, settings, toner, and a file sent straight to the printer are [ipp-tool.md](ipp-tool.md), not this library.

The library is `registration_server::native_print`. The build sequence is [native-print-implementation-plan.md](native-print-implementation-plan.md). It returns queues the operating system already has. It does not browse `_ipp._tcp`. It does not send `Print-Job` to a printer URI.

Linux and macOS compile the CUPS client. Windows compiles the spooler client. `scripts/release/pack.sh` builds the client for the machine it is run on, including the shared library, and the release workflow tests that client on each runner. `ipp-tool` is linked into `rust-reg` on every platform, because search discovers printers there. The CUPS client stays on Linux and macOS.

C and Python call the same list through `include/native_print.h` and `modules/native-print/native_print.py`. The symbols do not name CUPS or the spooler. The shared library that is built for the machine supplies the matching client.

## Record

`list` returns one record per queue:

| Field | Source on Linux and macOS |
|---|---|
| `name` | The CUPS queue name. |
| `device_uri` | The queue's device URI: `usb://`, `ipp://`, `ipps://`, `socket://`, `dnssd://`, or another backend. |
| `make_model` | `printer-make-and-model`. |
| `state` | `printer-state`. |
| `accepting` | Whether the queue accepts jobs. |
| `host` | Set when `device_uri` is `ipp://` or `ipps://`. The host from that URI, resolved to an address. Empty for USB and raw sockets. |

Make `zebra technologies` or `tsc` sets `kind` to `zpl`. Any other make sets `raster`. Model `ztc gx430t` and make `evolis` set `dpi` 300. Other ZPL queues set `dpi` 203 and `width_in` 4.1. Evolis sets `width_in` 2.125. A raster queue leaves `width_in` empty. `choices` are the PPD options except `PageSize`.

## Linux and macOS

`list` sends `CUPS-Get-Printers` to the local cupsd and reads the fields above. A queue exists, so `installed` is true. `connected` is left false for a non-IPP queue. Step 13 of [printing-implementation-plan.md](printing-implementation-plan.md) sets `connected` from `CUPS-Get-Devices` and installs a USB device that has no queue. An `ipp://` or `ipps://` queue is connected when the IPP tool has attributes for that address, as step 12 describes.

This library does not download a filter and does not rewrite a PPD.

## Windows

The same `list` function reads the Windows spooler. `EnumPrinters` supplies the queue name, the driver, the port, and the status. The driver string fills `make_model`, `kind`, `dpi`, and `width_in` the same way a CUPS make and model do. A port that is already `ipp://` or `ipps://`, or an `http` URL whose path contains `ipp`, sets `device_uri` and `host`. A `USB` port becomes `usb://` and leaves `host` empty. An `IP_` port becomes `socket://` on port 9100 and leaves `host` empty. Any other port, including WSD, becomes `winspool://`. The record fields stay the ones above, so this backend does not grow a second printer map. The steps are [native-print-implementation-plan.md](native-print-implementation-plan.md).

## What stays out

A USB printer and a `socket://` printer are native records only. Formats, toner, and `Set-Printer-Attributes` are not asked of them here. A badge for those queues still goes through the CUPS queue, as steps 16 and 18 describe.

An IPP printer found only by the browse, with no CUPS queue, is not invented by this library. The IPP tool holds it until the operator adds it.
