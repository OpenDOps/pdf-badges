# Manual test plan

Checks that need a person, a second machine, or a clean account, and that are not covered by `cargo test` or the admin tests. Each row says how to run it and what must be true. Set **Result** to `pass`, `fail` (with a note), or leave it empty while the check is open.

The server runs as the desktop user from a per-user prefix (macOS `~/Library/Application Support/rust-reg`, Linux `~/.local/share/rust-reg`, Windows `%LOCALAPPDATA%\rust-reg`). There is no root service; see [three-zip-upgrade-testing-plan.md](three-zip-upgrade-testing-plan.md#findings-during-the-proof).

## 1. Server stays reachable while the Mac looks asleep

**Why:** while the tray's server runs, the tray holds `caffeinate -i -w <server>` so macOS does not idle-sleep. The display may sleep and the screen may lock. Real system sleep (including closing a laptop lid without power and an external display) stops every process; that is expected.

**Setup:** tray open, server on a known port (here 8081), a second device on the same network.

| Step | Assert | Result |
|---|---|---|
| `pmset -g assertions \| grep caffeinate` | One `PreventUserIdleSystemSleep` line on behalf of the server's wrapper pid | |
| Lock the screen (Ctrl+Cmd+Q) and wait for the display to sleep | From the second device, `http://<mac-ip>:8081/health` returns `{"status":"ok"}` | |
| Leave it longer than the Energy "turn display off" / sleep timer (e.g. 30 min) | Still `{"status":"ok"}` from the second device; a badge station can register | |
| Quit the tray | The `caffeinate` assertion is gone (`pmset -g assertions`), and the Mac can idle-sleep again | |
| Laptop only: close the lid on battery, open after a minute | Server answers again after wake without restarting the tray (it was paused, not stopped) | |

## 2. Fresh install from the macOS pkg

**Why:** the pkg no longer installs a LaunchDaemon. Its `postinstall` (root, inside the installer's own password prompt) retires an old daemon, gives `reg-server.app` to the console user, and opens the tray. On the proof Mac this was reproduced by hand, not run from the package.

**Setup:** a Mac or a fresh macOS user account with no `~/Library/Application Support/rust-reg`. For the upgrade-from-root case, a Mac with an older pkg (≤ `alpha.11`) installed and its daemon running.

| Step | Assert | Result |
|---|---|---|
| Install `rust-reg-<version>.pkg` (Apple silicon) or `-intel.pkg` | One password prompt (the installer's); no other prompt follows | |
| After the installer finishes | `reg-server.app` opens by itself; menu-bar icon is the white template icon; no **Upgrade probe** item | |
| `ls -l /Applications/reg-server.app/Contents/MacOS/reg-server` | Owner is the console user, not root | |
| `ls /Library/LaunchDaemons \| grep opendops` | Nothing (an older daemon was booted out and its plist removed) | |
| `ps -axo user,command \| grep rust-reg` | `rust-reg-run` and `versions/<version>/rust-reg` run as the console user from `~/Library/Application Support/rust-reg` | |
| Open `http://localhost:8080/admin` (or the tray's port) | Login page; operator login works (a `device_id` is generated on first start) | |
| Quit, reopen, **Port…** | No password prompt for any of them | |
| Upgrade-from-root case: before installing, the old daemon was serving | After install the daemon is gone and the user server serves instead; the old `/Library/rust-reg/data` is not copied (operator logs in again) | |

## Done when

Every row has a result. Failures get a finding and a fix in the plan that owns the feature.
