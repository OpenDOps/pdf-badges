# Tray wrapper — step 12

Manual check for [porting.md](porting.md) step 12. The binary is `rust-reg-desk` (`rust-reg-desk.exe` on Windows), built from `desk/`. `scripts/release/pack.sh` puts it in the installer. `cargo test --manifest-path desk/Cargo.toml` does not open a display. This plan is the check that does.

The file under test is the installer from a published release, or a package just built by `pack.sh` on that machine. The upgrade zip does not carry the tray binary. An update leaves `bin/rust-reg-desk` as the first install wrote it.

Run the same checks on Linux, on macOS, and on Windows. A pass on one machine does not cover the others. macOS uses the pkg that matches that Mac: `rust-reg-<version>.pkg` on Apple silicon, `rust-reg-<version>-intel.pkg` on Intel.

Set **Status** to `not started`, `in progress`, or `done` on each machine after the checks are run.

## Summary

| Machine | How the operator opens the tray | Status |
|---|---|---|
| [Linux](#linux) | The `reg-server` application menu runs `/opt/rust-reg/bin/rust-reg-desk` | not started |
| [macOS](#macos) | `/Applications/reg-server.app` | not started |
| [Windows](#windows) | Start Menu shortcut `reg-server` | not started |

The session has to show a tray. A Linux machine with no status area, a Mac remote session with no menu bar, or a Windows session that cannot show the notification area cannot complete this plan.

## What the tray does

The server process has no window. The tray starts it when port 8080 is closed, and shows the listen address when the port is already open.

- The menu has the listen address and Quit. The address comes from the server line `registration-server http+ws host:port`. `0.0.0.0` and `::` are shown as `localhost`. The usual line is `localhost:8080`.
- The icon is a pencil and a person. On macOS it is a menu-bar template, and the app has no Dock icon.
- A second launch finds `rust-reg-desk.lock` and exits. One icon stays.
- An installed copy in `<prefix>/bin` starts `rust-reg-run --prefix <prefix>`. The macOS app starts `/Library/rust-reg/bin/rust-reg-run --prefix /Library/rust-reg`.
- When this tray started the server, Quit stops that process. When the installed service already held port 8080, Quit stops the service: `systemctl stop rust-reg.service` on Linux, `launchctl bootout system/com.opendops.rust-reg` on macOS, and `sc.exe stop rust-reg` on Windows.
- After Quit the icon is gone and nothing is listening on port 8080. Opening the tray again brings the icon back and the server back.

`<prefix>` is `/opt/rust-reg` on Linux, `/Library/rust-reg` on macOS, and `C:\Program Files\rust-reg` on Windows.

## Linux

[Back to summary](#summary)

Install `rust-reg_<version>_amd64.deb` with `dpkg -i` on a machine whose session shows a status icon. Log in to that graphical session. Open **reg-server** from the application menu. That runs `/usr/share/applications/rust-reg.desktop`.

### Check

| Check | Result |
|---|---|
| Icon | A status icon appears. The picture is a pencil and a person. |
| Menu | The menu shows `localhost:8080` and Quit. |
| Server | Port 8080 accepts a connection. The parent of the server is `/opt/rust-reg/bin/rust-reg-run`, and its arguments include `--prefix /opt/rust-reg`. |
| Second launch | Opening **reg-server** again does not add a second icon. One `rust-reg-desk` process is running. |
| Quit | The icon disappears. Port 8080 no longer accepts a connection. `rust-reg-desk` is not running. |
| Open again | The icon and `localhost:8080` come back. Port 8080 accepts a connection. |
| Service already running | With the tray quit, `systemctl start rust-reg` holds port 8080. Opening the tray shows `localhost:8080` and does not start a second server. Quit runs `systemctl stop rust-reg.service`. The icon is gone and the unit is inactive. |

## macOS

[Back to summary](#summary)

Install the pkg that matches this Mac. Open `/Applications/reg-server.app`.

### Check

| Check | Result |
|---|---|
| Icon | A menu-bar icon appears. The picture is a pencil and a person. No Dock icon appears. |
| Menu | The menu shows `localhost:8080` and Quit. |
| Server | Port 8080 accepts a connection. The server's parent is `/Library/rust-reg/bin/rust-reg-run`, and its arguments include `--prefix /Library/rust-reg`. |
| Second launch | Opening the app again does not add a second menu-bar icon. One `reg-server` process is running. |
| Quit | The menu-bar icon disappears. Port 8080 no longer accepts a connection. |
| Open again | The icon and `localhost:8080` come back. Port 8080 accepts a connection. |
| Service already running | With the tray quit, `launchctl bootstrap system /Library/LaunchDaemons/com.opendops.rust-reg.plist` holds port 8080. Opening the app shows `localhost:8080` and does not start a second server. Quit runs `launchctl bootout system/com.opendops.rust-reg`. The icon is gone and the job is not loaded. |

## Windows

[Back to summary](#summary)

Run `rust-reg-<version>.exe` with no arguments. It extracts into `C:\Program Files\rust-reg` and adds the Start Menu shortcut `reg-server`. Open that shortcut. A hidden notification-area icon counts as missing until it is visible in the tray.

### Check

| Check | Result |
|---|---|
| Icon | A notification-area icon appears. The picture is a pencil and a person. |
| Menu | The menu shows `localhost:8080` and Quit. |
| Server | Port 8080 accepts a connection. The server's parent is `C:\Program Files\rust-reg\bin\rust-reg-run.exe`, and its arguments include `--prefix` and `C:\Program Files\rust-reg`. |
| Second launch | Opening `reg-server` again does not add a second icon. One `rust-reg-desk.exe` process is running. |
| Quit | The icon disappears. Port 8080 no longer accepts a connection. `rust-reg-desk.exe` is not running. |
| Open again | The icon and `localhost:8080` come back. Port 8080 accepts a connection. |
| Service already running | With the tray quit, `sc start rust-reg` holds port 8080. Opening the shortcut shows `localhost:8080` and does not start a second server. Quit runs `sc stop rust-reg`. The icon is gone and the service is stopped. |

## Done when

The seven checks have been run on Linux, the seven checks have been run on macOS, and the seven checks have been run on Windows. Each machine's row in the summary is `done`. Step 12 of [porting.md](porting.md) is accepted only then.
