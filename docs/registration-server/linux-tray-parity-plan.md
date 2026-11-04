# Linux tray parity with macOS

Shared process control lives in `desk/src/server.rs` (start, restart, port remember, listen URL, Quit kill of wrapper + server). Platform UI is `desk/src/linux.rs` (ksni + zenity) vs `desk/src/macos.rs` (tray-icon + AppKit).

Deb install launches `/opt/rust-reg/bin/rust-reg-desk` from `rust-reg.desktop`.

Each step is one change in `desk/`. Set **Status** to `not started`, `in progress`, or `done`. A later step keeps earlier behaviour working. Manual checks need a graphical Linux session with a status area.

**Status: done.** Steps 1 through 7 are `done`. Windows parity is out of scope.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Dialog tool chain](#step-1-dialog-tool-chain) | Prefer zenity, then kdialog, then yad; stderr fallback when none | done |
| [2. Port dialog labels](#step-2-port-dialog-labels) | Title **Port**, prefill, OK label **Apply and restart** when the tool allows | done |
| [3. Async dialogs](#step-3-async-dialogs) | Port and error dialogs run off the menu callback so the tray loop stays alive | done |
| [4. Quit during dialog](#step-4-quit-during-dialog) | Quit kills the open dialog child, stops the server, tears down the tray | done |
| [5. Single Port dialog](#step-5-single-port-dialog) | Second **Port…** focuses or ignores; no second entry process | done |
| [6. Menu refresh](#step-6-menu-refresh) | After restart, address / failed line updates without a stale label | done |
| [7. Manual matrix](#step-7-manual-matrix) | Extend [tray-wrapper-testing-plan.md](tray-wrapper-testing-plan.md) and run once on Linux | done |

## Already on Linux

| Behaviour | How |
|---|---|
| StatusNotifier tray icon (pencil + person) | `ksni` |
| Menu: address, **Port…**, **Quit** | `StandardItem` |
| Click live address opens browser | `xdg-open` via `server::open_listen` |
| Startup / restart failure shows an error dialog | `zenity --error` (blocking) |
| **Port…** asks for a port and restarts | `zenity --entry` then `session.restart` |
| Status `failed on port N` when start fails | shared `Session` |
| Remembered port across launches | shared durable `desk.port` under state dir (not runtime lock dir) |
| Second instance exits quietly | shared lock file |
| Quit stops wrapper + registration-server | shared `Session::stop` |

## Gaps vs macOS

| Gap | macOS | Linux today |
|---|---|---|
| Port dialog shape | Non-modal panel: field, **Cancel**, **Apply and restart** (enabled only when the number differs) | Modal `zenity --entry`; OK with an unchanged port is a no-op via `replacement_port` |
| Quit while Port is open | Closes the panel and quits | Zenity blocks the activate path; Quit waits until the entry closes |
| Bring Port to front | Second **Port…** focuses the open panel | Always opens another zenity |
| Error dialog chrome | Centered critical `NSAlert`; Quit does not dismiss it | `zenity --error`; fine if zenity is installed |
| No zenity | N/A | Silent failure: no error / no port UI |
| Address line while failed | Disabled menu item | Same intent (`listen_url` gate); menu rebuilt when opened |

## Out of scope

- Native GTK/Qt widgets (unless the dialog tool chain in step 1 proves too weak).
- Matching macOS AppKit visuals.
- Changing `/opt/rust-reg` vs Application Support layout (separate install work).
- Windows parity (`desk/src/windows.rs`); follow-up plan later.

---

## Step 1. Dialog tool chain

**Status:** done

**Files:** `desk/src/linux.rs`

**Work:**

1. Add a small helper that picks the first available of `zenity`, `kdialog`, `yad` (`which` / `Command` probe once per process).
2. Route `show_error` and the port prompt through that helper with tool-specific argv.
3. When no tool exists: write one line to stderr; keep the tray up; leave **Port…** in the menu but make activate a no-op (stderr hint).

**Done when:** with zenity installed, error and port dialogs still open; with all three tools missing (PATH stripped in a manual check), the tray starts and stderr explains the missing dialog tool. `cargo test --manifest-path desk/Cargo.toml` stays green.

**Notes:** Probe is `OnceLock` + `which`. Automated dialog-tool tests are deferred to a Linux session; host `cargo test` covers shared `server` only.

---

## Step 2. Port dialog labels

**Status:** done  
**Depends on:** step 1

**Files:** `desk/src/linux.rs`

**Work:**

1. Keep dialog title **Port** and prefill the current port.
2. Where the tool supports it (`zenity --ok-label`, kdialog/yad equivalents), set the primary button to **Apply and restart**. Cancel / Escape remains cancel.
3. Keep `server::replacement_port`: OK with the same port applies nothing.

**Done when:** changing 8080 → 8081 restarts; menu shows `localhost:8081` or `failed on port 8081` plus an error dialog. Unchanged port + OK does not restart.

**Notes:** zenity `--ok-label`, kdialog `--ok-label`, yad `--button Apply and restart:0`. Manual Apply/Cancel checks deferred with step 7.

---

## Step 3. Async dialogs

**Status:** done  
**Depends on:** step 1

**Files:** `desk/src/linux.rs`

**Work:**

1. Stop calling blocking zenity inside `StandardItem` `activate` for Port and for error-after-restart.
2. Extend `Action` (or a sibling channel) so the menu only requests “open port dialog” / “show error”.
3. Spawn the dialog child from the main `run` loop and complete `session.restart` / show failure there when the child exits.

Startup error before the tray is up may still block once (same as today’s first `show_error`), or move to the same async path after `spawn` of the tray — pick one and keep it consistent.

**Done when:** while a Port dialog is open, the process still services other tray menu events (verified together with step 4). No dialog runs inside `activate` except a non-blocking spawn.

**Notes:** Menu sends `AskPort` / `ShowError`; `run` owns the `Child` and polls with `try_wait`. Startup failure uses the same async path after tray `spawn`. Quit-while-open kill is a small prelude to step 4; full SIGTERM + ignore-as-apply acceptance stays on step 4. Manual “menu while dialog open” check deferred with step 4/7.

---

## Step 4. Quit during dialog

**Status:** done  
**Depends on:** step 3

**Files:** `desk/src/linux.rs`

**Work:**

1. Keep the dialog child `Child` (or PID) where `run` can see it.
2. On **Quit**: SIGTERM the dialog child if any, ignore its exit as a port apply, call `session.stop()`, shut down ksni, return.
3. Do not treat Quit-killed port zenity as **Apply and restart**.

**Done when:** open **Port…**, choose **Quit** without closing the dialog → dialog gone, tray icon gone, listen port free, no new server start.

**Notes:** `terminate_pending` sends SIGTERM, waits briefly, then SIGKILL. Pending is taken before terminate so `apply_finished` never runs for a Quit-killed Port dialog. Manual Quit-while-Port check deferred to step 7.

---

## Step 5. Single Port dialog

**Status:** done  
**Depends on:** step 3

**Files:** `desk/src/linux.rs`

**Work:**

1. If a port dialog child is already running, do not spawn a second one.
2. Best-effort raise/focus (`wmctrl` / `xdotool` when present); otherwise ignore the second **Port…** click.
3. Clear the tracked child when the dialog exits (OK, Cancel, or Quit).

**Done when:** two quick **Port…** clicks leave a single dialog process (`pgrep` / `ps`).

**Notes:** Second **Port…** calls `raise_dialog` (`xdotool` by PID, else `wmctrl -a Port`). No second spawn. Manual `pgrep` check deferred to step 7.

---

## Step 6. Menu refresh

**Status:** done  
**Depends on:** step 2 (restart path)

**Files:** `desk/src/linux.rs`

**Work:**

1. On GNOME and KDE, after `session.restart`, confirm the next menu open shows the new address or `failed on port N`.
2. If the label stays stale until tray restart, call ksni’s refresh/update API after mutex writes (or equivalent workaround).
3. No change if both desktops already rebuild `menu()` correctly.

**Done when:** after Apply to a free port, opening the menu once shows the live `localhost:N` without relaunching the desk.

---

## Step 7. Manual matrix

**Status:** done  
**Depends on:** steps 1–6

**Files:** [tray-wrapper-testing-plan.md](tray-wrapper-testing-plan.md), this file

**Work:**

1. Add a Linux subsection (or table rows) for the checks below without weakening macOS/Windows rows that still describe older “attach to open 8080” behaviour — note current “port taken = error” where those docs conflict, or update Linux-only rows to the new behaviour.
2. Run the matrix once on a graphical Linux session; set this step to `done` and mark the Linux summary row in the tray testing plan when that plan’s Linux status is updated.

| Check | Expect |
|---|---|
| Port taken | Error dialog; menu `failed on port N`; tray stays up |
| Port… Apply | New port; address clickable when live |
| Port… Cancel | No restart |
| Quit during Port… | Dialog and tray exit; port free |
| Address click | Browser opens `http://localhost:N` |
| No dialog tool | Tray still runs; stderr explains missing tool |
| Second Port… | One dialog only |

**Done when:** every row above has been run once; this step’s Status is `done`.

---

## Done when

The summary table shows steps 1–7 as `done`. Linux tray Port / error / Quit behaviour matches the acceptance lines in those steps. macOS remains the reference for non-modal Apply enable; Linux keeps zenity-class dialogs unless this plan is revised.
