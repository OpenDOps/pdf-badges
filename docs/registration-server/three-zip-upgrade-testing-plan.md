# Three-zip upgrade — step 16 implementation and proof

Build and test sequence for [release-implementation-plan.md](release-implementation-plan.md) **step 16**. That step’s Work / Test scenarios stay the product contract; this file is the ordered implementation path plus the observable Mac → Linux proof.

Each step below is one change (or one manual matrix). The tests named in the step land with it and stay green. A later step keeps earlier behaviour. Set **Status** to `not started`, `in progress`, or `done`.

This plan is done when steps 1 through 13 are `done`.

**Steps 1–10 are in tree.** [Step 11](#step-11-macos-proof) is running: Phase 0 done, Phase 1 publishing A/B/C as `alpha.9` / `10` / `11`. Bugs found on the way are listed under [Findings](#findings-during-the-proof).

## Summary

| Step | What it covers | Test plan | Status |
|---|---|---|---|
| [1. Pack three zips](#step-1-pack-three-zips) | `pack.sh` writes server / wrapper / tray zips + file lines in `SHA256SUMS` | [§ Test](#step-1-pack-three-zips) | done |
| [2. Select zips by digest](#step-2-select-zips-by-digest) | `POST /api/update` fetches only zips whose installed file digest differs | [§ Test](#step-2-select-zips-by-digest) | done |
| [3. Unpack parts](#step-3-unpack-parts) | Server / wrapper / tray zips land in `versions/<to>/` (or live tree copy on skip) | [§ Test](#step-3-unpack-parts) | done |
| [4. Replace tray](#step-4-replace-tray) | After `current`, replace `bin/rust-reg-desk` (and macOS app binary) like the wrapper | [§ Test](#step-4-replace-tray) | done |
| [5. Publish assets](#step-5-publish-assets) | CI / release upload three zips; drop the single `*-linux.zip` / `*-macos.zip` upgrade asset | [§ Test](#step-5-publish-assets) | done |
| [6. Automated parts suite](#step-6-automated-parts-suite) | `cargo test --lib registration_server::update::tests::parts` | [§ Test](#step-6-automated-parts-suite) | done |
| [7. Probe logs](#step-7-probe-logs) | Wrapper and server log `upgrade-probe-*-{LETTER}` | [§ Test](#step-7-probe-logs) | done |
| [8. Tray markers](#step-8-tray-markers) | Distinct icon + menu item **Upgrade probe** (B); removed or changed in C | [§ Test](#step-8-tray-markers) | done |
| [9. Frontend `/testpath`](#step-9-frontend-testpath) | Hardcoded HTML + script probe letter in the form (or admin) tree | [§ Test](#step-9-frontend-testpath) | done |
| [10. Marker packs A/B/C](#step-10-marker-packs-abc) | Three installers + three-zip sets with letters A → B → C | [§ Test](#step-10-marker-packs-abc) | done |
| [11. macOS proof](#step-11-macos-proof) | Full observable upgrade matrix on Mac | [§ Test](#step-11-macos-proof) | in progress (Phase 0 done, Phase 1 running) |
| [12. Linux proof](#step-12-linux-proof) | Tray + wrapper markers; server smoke only | [§ Test](#step-12-linux-proof) | not started |
| [13. Cleanup](#step-13-cleanup) | Remove or gate probes; leave shipping icons/menus | [§ Test](#step-13-cleanup) | not started |

Dependency sketch:

```text
1 → 2 → 3 → 4 → 5 → 6
                ↘
7 → 8 → 9 → 10 → 11 → 12 → 13
         ↗ (needs 1–6 for real updates)
```

Steps **1–6** are product code (release step 16). Steps **7–13** are the proof that upgrades are visible. Do not start [11](#step-11-macos-proof) until **1–6** and **7–10** are `done` (or intentionally skip markers and only assert digests — weaker).

### Marker cheat sheet (used by steps 7–12)

| Component | Build B (from A) | Build C (from B) |
|---|---|---|
| Tray | New icon + menu **Upgrade probe** | Icon ≠ B; **Upgrade probe** gone |
| Wrapper | Log `upgrade-probe-wrapper-B` | Log `upgrade-probe-wrapper-C` |
| Server | Log `upgrade-probe-server-B` | Log `upgrade-probe-server-C` |
| Frontend | `/testpath` shows probe `B` | Probe `C` or route removed |

### Retest server on Linux?

**No full server/frontend matrix on Linux** if [step 11](#step-11-macos-proof) already proved `/testpath` and server probe logs. [Step 12](#step-12-linux-proof) still smokes listen + version tag. Re-run `/testpath` on Linux only if Mac failed or a Linux/arch pack bug is suspected.

---

## Step 1. Pack three zips

[Back to summary](#summary)

**Status:** done  
**Depends on:** release plan steps 12–15 already done  
**Unlocks:** [2](#step-2-select-zips-by-digest), [5](#step-5-publish-assets), [10](#step-10-marker-packs-abc)

### Work

1. Change `scripts/release/pack.sh` so the upgrade side writes **three** deflated zips (names without `v`), not one `rust-reg-<version>-<os>.zip`:

| Platform | Server | Wrapper | Tray |
|---|---|---|---|
| Linux amd64 | `…-linux-server.zip` | `…-linux-wrapper.zip` | `…-linux-tray.zip` |
| Linux arm64 | `…-linux-arm64-server.zip` | `…-linux-arm64-wrapper.zip` | `…-linux-arm64-tray.zip` |
| Apple silicon | `…-macos-server.zip` | `…-macos-wrapper.zip` | `…-macos-tray.zip` |
| Intel macOS | `…-macos-intel-server.zip` | `…-macos-intel-wrapper.zip` | `…-macos-intel-tray.zip` |
| Windows | `…-windows-server.zip` | `…-windows-wrapper.zip` | `…-windows-tray.zip` |

2. Server zip: `rust-reg` (+ `.exe` on Windows), `admin/`, `form/`, `RELEASE`. Wrapper zip: `rust-reg-run` only. Tray zip: `rust-reg-desk` only.
3. Print `SHA256SUMS` lines for each zip **and** each installed file (platform-qualified names: `rust-reg-macos`, `rust-reg-run-macos`, `rust-reg-desk-macos`, etc., as in the release plan).
4. First-install packages (deb / pkg / exe) unchanged: still ship `bin/rust-reg-run` and `bin/rust-reg-desk`.

### Test plan

| Scenario | Assert |
|---|---|
| `pack_writes_three_zips` | After `pack.sh` on this OS, the three zip paths exist; the old single `*-linux.zip` / `*-macos.zip` upgrade asset is absent. |
| `server_zip_layout` | Unzip server zip → `rust-reg`, `admin/`, `form/`, `RELEASE`; no `rust-reg-run` / `rust-reg-desk`. |
| `wrapper_and_tray_are_single_files` | Wrapper zip is only the run binary; tray zip is only the desk binary. |
| `sha256sums_lists_zips_and_files` | Printed lines verify with `shasum -a 256 -c` / `sha256sum -c` for all six (3 zip + 3 file) names. |

### Done when

Local `bash scripts/release/pack.sh` on Mac (and later Linux CI) satisfies the table. Status → `done`. Continue to [step 2](#step-2-select-zips-by-digest).

Done. `scripts/release/upgrade-zips.sh` writes the three zips and prints zip + file-key lines. `pack.sh` calls it with the platform tag (`macos`, `macos-intel`, `linux`, `linux-arm64`, `windows`). `bash scripts/release/test-upgrade-zips.sh` covers `pack_writes_three_zips`, `server_zip_layout`, `wrapper_and_tray_are_single_files`, `sha256sums_lists_zips_and_files`, and skipping the tray when desk is absent. CI copies only SHA256SUMS names that exist as files under `target/release/` (file-key lines are digests only).

---

## Step 2. Select zips by digest

[Back to summary](#summary)

**Status:** done  
**Depends on:** [1](#step-1-pack-three-zips)  
**Unlocks:** [3](#step-3-unpack-parts)

### Work

1. `POST /api/update` downloads `SHA256SUMS` first.
2. Hash installed server (`versions/<live>/rust-reg`), wrapper (path from step 15), tray (`<prefix>/bin/rust-reg-desk`).
3. Compare each to the **file** line (not the zip line). Matching digest → skip that zip. Differing → download only that zip and verify with the **zip** line (existing checksum step).
4. Update `asset_name` / download helpers: no single `*-linux.zip` for upgrades.

### Test plan

| Scenario | Assert |
|---|---|
| `a_matching_tray_is_not_downloaded` | Stand-in sees `SHA256SUMS`, never the tray zip; `bin/rust-reg-desk` unchanged. |
| `a_new_tray_is_downloaded` | Tray digest differs → tray zip fetched; matching wrapper zip not requested. |
| `a_matching_wrapper_is_not_downloaded` | Wrapper digest matches → wrapper zip not fetched. |
| `the_server_zip_skips_when_the_binary_matches` | Live `rust-reg` matches file line → server zip not fetched. |
| `a_bad_zip_digest_downloads_nothing_else` | Bad tray zip → `checksum_mismatch`; other pending zips not fetched. |

(Align names with release plan step 16; implement under `registration_server::update::tests::parts`.)

### Done when

The scenarios above fail until coded, then pass in isolation (or with [6](#step-6-automated-parts-suite)). Status → `done`. Continue to [step 3](#step-3-unpack-parts).

Done. `download_release` / `fetch_parts` read `SHA256SUMS`, compare installed file digests via `part_file_key`, and download only mismatched zips. Covered by `registration_server::update::tests::parts` (step 2 rows).

---

## Step 3. Unpack parts

[Back to summary](#summary)

**Status:** done  
**Depends on:** [2](#step-2-select-zips-by-digest)  
**Unlocks:** [4](#step-4-replace-tray)

### Work

1. Fetched server zip unpacks into `versions/<new>/` as today (no run/desk binaries required in that zip).
2. If server zip was skipped, copy `rust-reg`, `admin/`, `form/`, `RELEASE` from the live tree into `versions/<new>/`.
3. Fetched wrapper zip → `versions/<new>/rust-reg-run` only. Fetched tray zip → `versions/<new>/rust-reg-desk` only. Skipped part → that name absent in the new tree.

### Test plan

| Scenario | Assert |
|---|---|
| `server_unpack_without_tray_wrapper` | After server-only fetch, `versions/<to>/` has the four server entries and no desk/run unless copied from another zip. |
| `wrapper_zip_only_adds_run` | Wrapper fetch writes only `rust-reg-run` under `versions/<to>/`. |
| `tray_zip_only_adds_desk` | Tray fetch writes only `rust-reg-desk` under `versions/<to>/`. |
| `skipped_server_copies_live_tree` | Skip server zip → `versions/<to>/rust-reg` (and admin/form/RELEASE) equal live bytes. |

### Done when

Scenarios pass. Status → `done`. Continue to [step 4](#step-4-replace-tray).

Done. `unpack_parts` builds `versions/<to>/` from fetched zips; a skipped server zip copies only the four live server entries. `install` stages via `unpack_parts`. Covered by `registration_server::update::tests::parts` (step 3 rows).

---

## Step 4. Replace tray

[Back to summary](#summary)

**Status:** done  
**Depends on:** [3](#step-3-unpack-parts), release step 15 (wrapper replace)  
**Unlocks:** [6](#step-6-automated-parts-suite), manual [11](#step-11-macos-proof) / [12](#step-12-linux-proof)

### Work

1. After phase is `current`, keep replacing the wrapper from `versions/<to>/rust-reg-run` when present and digests differ.
2. Same pass: compare `<prefix>/bin/rust-reg-desk` to `versions/<to>/rust-reg-desk`. Missing new file → leave installed tray. Same digest → leave. Different → replace, fsync file + directory.
3. macOS: also replace `/Applications/reg-server.app/Contents/MacOS/reg-server` when that path exists.
4. Rolled-back phase does not replace tray or wrapper. Failed copy leaves previous file and phase `current`.

### Test plan

| Scenario | Assert |
|---|---|
| `a_new_tray_is_downloaded` (finish) | After `current`, `bin/rust-reg-desk` matches `versions/<to>/rust-reg-desk`. |
| `rollback_does_not_replace_the_tray` | Phase `rolled_back` → installed tray bytes unchanged. |
| `matching_tray_digest_leaves_bin` | Same digest → no rewrite of `bin/rust-reg-desk`. |

### Done when

Scenarios pass. Status → `done`. Continue to [step 5](#step-5-publish-assets) and [step 6](#step-6-automated-parts-suite).

Done. `replace_tray` runs with the wrapper after phase `current` (supervise + `note_listening`). Missing `versions/<to>/rust-reg-desk` leaves `bin/`; matching digest leaves the inode; macOS also updates `reg-server.app` when present. Covered in `registration_server::update::tests::parts` (`a_new_tray_is_downloaded` finish, `rollback_does_not_replace_the_tray`, `matching_tray_digest_leaves_bin`, plus failed-copy / missing-source / app-path cases).

---

## Step 5. Publish assets

[Back to summary](#summary)

**Status:** done  
**Depends on:** [1](#step-1-pack-three-zips)  
**Unlocks:** live GitHub offers for [11](#step-11-macos-proof) (optional local static server also OK)

### Work

1. Update `.github/workflows/release.yml` publish merge list / pack outputs for three zips per platform (including `linux-arm64-*`).
2. Stop uploading the old single upgrade zip name.
3. Docs already describe three zips; keep [porting.md](porting.md) / release plan asset tables in sync if names drift (e.g. arm64 suffix).

### Test plan

| Scenario | Assert |
|---|---|
| `workflow_packs_three_per_platform` | A dry run or CI log lists three zip artifacts per selected OS; no single `rust-reg-<ver>-macos.zip` upgrade (installer pkg/deb/exe still uploaded). |
| `sha256sums_on_release` | Published `SHA256SUMS` contains zip + file lines for each packed platform. |

### Done when

CI or local pack+upload checklist passes. Status → `done`.

Done. `release.yml` asserts each platform packed the three `*-server/wrapper/tray.zip` files, rejects legacy single upgrade zips, merges `linux-arm64` into `SHA256SUMS`, and deletes leftover single-zip assets when re-uploading a tag. `porting.md` publish/install tables name the three zips (including arm64).

---

## Step 6. Automated parts suite

[Back to summary](#summary)

**Status:** done  
**Depends on:** [2](#step-2-select-zips-by-digest)–[4](#step-4-replace-tray)  
**Unlocks:** [11](#step-11-macos-proof) (required gate)

### Work

1. Land / finish `registration_server::update::tests::parts` covering release plan step 16 scenarios.
2. Keep steps 1–15 updater tests green.

### Test plan

| Scenario | Assert |
|---|---|
| `parts_module_green` | `cargo test --lib registration_server::update::tests::parts` passes. |
| `prior_update_tests_green` | Existing `update::tests` modules still pass. |

### Done when

Both commands green. Status → `done`. **Product step 16 code path is testable;** markers still needed for visible proof ([7](#step-7-probe-logs)+).

Done. `registration_server::update::tests::parts` covers release step 16 rows (select / unpack / replace tray+wrapper, including matching-wrapper leave and skipped-server live-tree finish). `cargo test --lib registration_server::update::tests::parts` and `cargo test --lib registration_server::update::tests::` are green (14 + 84).

---

## Step 7. Probe logs

[Back to summary](#summary)

**Status:** done  
**Depends on:** none (can start in parallel with 1–6 on a branch)  
**Unlocks:** [10](#step-10-marker-packs-abc), [11](#step-11-macos-proof), [12](#step-12-linux-proof)

### Work

1. `src/bin/rust-reg-run.rs`: on start, log `rust-reg-run: upgrade-probe-wrapper-{LETTER}`.
2. Registration server after listen (or immediately before): `registration-server: upgrade-probe-server-{LETTER}`.
3. Letter from `RUST_REG_UPGRADE_PROBE` env or a const flipped per marker commit (prefer env for one branch + three packs).

### Test plan

| Scenario | Assert |
|---|---|
| `wrapper_probe_in_stderr` | Running the wrapper with probe `B` prints `upgrade-probe-wrapper-B` once at start. |
| `server_probe_after_listen` | Server with probe `B` prints `upgrade-probe-server-B` when the listen socket is up. |
| `probe_off_is_silent` | Unset / empty probe → no `upgrade-probe-` lines (shipping default). |

### Done when

Scenarios pass (unit or manual one-shot). Status → `done`. Continue to [step 8](#step-8-tray-markers).

Done. `RUST_REG_UPGRADE_PROBE` (one ASCII letter) drives `emit_upgrade_probe`. The wrapper logs `rust-reg-run: upgrade-probe-wrapper-{LETTER}` at start; the server logs `registration-server: upgrade-probe-server-{LETTER}` after the listen socket binds. Covered by `registration_server::update::tests::upgrade_probe` (`wrapper_probe_in_stderr`, `server_probe_after_listen`, `probe_off_is_silent`).

---

## Step 8. Tray markers

[Back to summary](#summary)

**Status:** done  
**Depends on:** [7](#step-7-probe-logs) optional  
**Unlocks:** [10](#step-10-marker-packs-abc), [11](#step-11-macos-proof), [12](#step-12-linux-proof)

### Work

1. When probe letter is `B`: show menu item **Upgrade probe** (no-op or one-line dialog) on macOS and Linux trays.
2. When letter is `B` (or `C`): use a distinct icon / tint so B ≠ A and C ≠ B.
3. Letter `C` or unset: no **Upgrade probe** item; shipping or C-specific icon.

### Test plan

| Scenario | Assert |
|---|---|
| `probe_b_shows_menu_item` | Desk built with probe B: menu contains **Upgrade probe**. |
| `probe_c_hides_menu_item` | Probe C or unset: item absent. |
| `icons_differ_by_letter` | B icon bytes or tint ≠ A; C ≠ B (visual or hash check of embedded/theme asset). |

### Done when

Scenarios checked on at least one OS in dev. Status → `done`. Continue to [step 9](#step-9-frontend-testpath).

Done. `RUST_REG_UPGRADE_PROBE` drives desk tray markers: letter `B` adds menu **Upgrade probe** (info dialog) on macOS/Linux/Windows; `B`/`C` use distinct icon tints (macOS non-template color; Linux/Windows RGBA — theme PNGs for deb come from `--write-icon` under the same env). Covered by `desk::server::tests` (`probe_b_shows_menu_item`, `probe_c_hides_menu_item`, `icons_differ_by_letter`).

---

## Step 9. Frontend `/testpath`

[Back to summary](#summary)

**Status:** done  
**Depends on:** form (or admin) pack path used by server zip  
**Unlocks:** [10](#step-10-marker-packs-abc), [11](#step-11-macos-proof)

### Work

1. Add route `/testpath` that renders hardcoded HTML plus a value from a small module/script `UPGRADE_PROBE` (`"A"` / `"B"` / `"C"`).
2. Gate behind the same probe env/flag when possible so shipping builds omit the route.
3. Ensure the built `form/` (or `admin/`) tree inside the **server** zip carries this page.

### Test plan

| Scenario | Assert |
|---|---|
| `testpath_shows_probe_letter` | With probe B, `GET /testpath` (or client route) body contains `B` and fixed heading text. |
| `testpath_absent_when_gated_off` | Shipping / unset probe: 404 or no route module. |
| `in_server_zip` | Unzipped `form/` (or `admin/`) from server zip contains the built `/testpath` assets when probe B was packed. |

### Done when

Scenarios pass. Status → `done`. Continue to [step 10](#step-10-marker-packs-abc).

Done. `GET /testpath` serves `form/testpath.html` (404 when missing). Pack writes that page plus `form/upgrade-probe.js` (`export const UPGRADE_PROBE = "…"`) only when `RUST_REG_UPGRADE_PROBE` is one letter (`scripts/release/write-testpath.sh`, same content as `write_upgrade_probe_testpath`). Covered by `registration_server::update::tests::testpath`, `admin_pages::serve_admin::{testpath_shows_probe_letter,testpath_absent_when_gated_off}`, and `test-upgrade-zips.sh` `in_server_zip`.

---

## Step 10. Marker packs A/B/C

[Back to summary](#summary)

**Status:** done  
**Depends on:** [1](#step-1-pack-three-zips)–[6](#step-6-automated-parts-suite), [7](#step-7-probe-logs)–[9](#step-9-frontend-testpath)  
**Unlocks:** [11](#step-11-macos-proof), [12](#step-12-linux-proof)

### Work

1. Produce three version tags (e.g. `2.0.0-alpha.160` / `161` / `162` — **numeric** prerelease only; `16a`-style suffixes are rejected by CI / `parse_version`) or local `REGISTRATION_RELEASE_TAG` values.
2. **A:** first-install package; probes off or letter A; shipping tray.
3. **B:** all three zips differ; probes B; tray icon + **Upgrade probe**; `/testpath` = B.
4. **C:** probes C; remove **Upgrade probe**; icon ≠ B; `/testpath` = C or removed.
5. Host assets: GitHub pre-release + `REGISTRATION_UPDATE_PRERELEASE=1`, or a static directory stand-in the updater can read.

`RUST_REG_UPGRADE_PROBE` is baked at compile time into server/wrapper (`build.rs`) and desk (`desk/build.rs`) so file digests differ and upgrades fetch all three zips without relying on launchd/systemd env. Runtime env still overrides when set. Pack also writes form `/testpath` and, on Linux with a probe letter, forces `--write-icon` tints.

### Test plan

| Scenario | Assert |
|---|---|
| `three_installers_exist` | A pkg/deb exists; B and C each have three zips + `SHA256SUMS`. |
| `digests_differ_where_expected` | B tray/wrapper/server file lines ≠ A; C tray line ≠ B. |
| `b_zip_contains_markers` | B tray binary / server form tree contain probe B markers (strings or route). |

### Done when

Artifacts ready for Mac install. Status → `done`. Continue to [step 11](#step-11-macos-proof).

Done. Fixture packs + asserts: `bash scripts/release/test-marker-packs.sh` (`three_installers_exist`, `digests_differ_where_expected`, `b_zip_contains_markers`). Real A/B/C packs: see `scripts/release/pack-markers.sh` (bump version, set `RUST_REG_UPGRADE_PROBE`, run `pack.sh` per letter).

---

## Step 11. macOS proof

[Back to summary](#summary)

**Status:** in progress — Phase 0 done; Phase 1: A + B published (C after A → B); Phase 2 done; Phase 3 A → B done except the Port… check on B  
**Depends on:** [6](#step-6-automated-parts-suite), [10](#step-10-marker-packs-abc)  
**Unlocks:** [12](#step-12-linux-proof) (recommended), [13](#step-13-cleanup)

Prefix: `/Library/rust-reg` (pkg). Tray: `/Applications/reg-server.app`. Root-side steps use [mac-proof.sh](../../scripts/release/mac-proof.sh) (`install`, `fixup`, `port`, `restart`).

Versions: **A = `2.0.0-alpha.9`, B = `alpha.10`, C = `alpha.11`** (numeric prerelease only — not `9a`). The first attempt (`alpha.6` A / `alpha.7` B) was published but superseded: those pkgs/trays carry the bugs listed under [Findings](#findings-during-the-proof). Clients need `REGISTRATION_UPDATE_PRERELEASE=1` (`mac-proof.sh install` sets it).

**Order matters:** the updater offers the *newest* release, so publish C only after A → B is verified (otherwise A jumps straight to C).

### Findings during the proof

Bugs found while running Phases 1–2 and fixed on `main` (each with tests):

| Bug | Fix | Commit |
|---|---|---|
| `workflow_dispatch` publish failed: `gh release create --verify-tag` with no `release-*` tag | Publish creates the tag at the built SHA (`--target`); no duplicate tag-push run | `c37d89e1` |
| `hdiutil` stdout leaked ~30 junk lines into `SHA256SUMS` (`shasum -c` warns; updater ignored them) | `hdiutil` output → stderr | `979045f2` |
| macOS pkg shipped no `updates/live` → `rust-reg-run: updates/live` crash loop | pkg writes `updates/live` (like deb/Windows) | `5e7cd9bb` |
| pkg relocated `reg-server.app` onto an older copy (`~/Applications`), out of reach of `replace_tray` | App bundle marked non-relocatable | `5e7cd9bb` |
| Tray adopted **any** listener on its port (Docker on 8080 shown as running) | Adopt only when `/health` answers like rust-reg; else “port N is in use by another program; the server did not start” | `c99571d4` |
| Quit / **Port…** sent TERM/KILL to **every** listener on the port (would kill Docker’s forwarder) | `stop_port` signals only `rust-reg` / `rust-reg-run` | `c99571d4` |
| Tray remembered a port before it started (a busy port stuck across launches) | `desk.port` written only after a successful start / adopt | `5b3090b8` |
| Service (launchd / systemd / Windows) ignored the tray port | Wrapper reads `<prefix>/data/port`; with a service installed, **Port…** writes it and restarts the service (admin prompt), tray starts on the service port | `5b3090b8` |
| **Port…** and Quit asked for an admin password every time; Quit unloaded the service, and the reopened tray then ran its own server as the user (`credentials.yml: Permission denied`) | The wrapper (root) makes `data/port` user-writable (macOS `staff` 664, Linux 666, Windows `Users` modify) and restarts the server when it changes; `data/port` now wins over `RUST_REG_HTTP`. **Port…** only writes the file (elevated restart only as a fallback for old installs). Quit leaves the service running. A tray that finds the service down says so instead of starting a copy | this commit |
| Even without per-action prompts, a root service still needed admin rights to start, stop, or recover (the old B tray's Quit unloaded it again) | **No root at runtime.** The tray runs the server as the desktop user from a per-user prefix (macOS `~/Library/Application Support/rust-reg`, Linux `~/.local/share/rust-reg`, Windows `%LOCALAPPDATA%\rust-reg`). The installed copy (`/Library/rust-reg`, `/opt/rust-reg`, `C:\Program Files\rust-reg`) only seeds it: first run copies it without `data/`, a newer installed version refreshes `bin/` and `versions/`. Packages no longer install a LaunchDaemon / systemd unit / Windows service and retire an old one; the macOS pkg gives `reg-server.app` to the console user so self-update can replace it. macOS: `caffeinate -i -w <server>` keeps the system from idle sleep while the server runs (display may sleep). | next release |
| Tests wrote to the real `/Applications/reg-server.app` / assumed no `/Library/rust-reg` | Test-only guards | `5b3090b8` |

Known environment note (this Mac): Docker `expocab-node` holds **8080** (and other containers 8090/8100/8110) — use port **8081** for the proof.

### Work (manual) — phases

#### Phase 0 — Preflight (automated) — **done**

All green, run one by one (`test-upgrade-zips.sh` 6/6, `test-marker-packs.sh` 3/3, `parts` 14, `upgrade_probe` 4, `testpath` 2, `admin_pages testpath_` 2, desk `probe_` 3 + `icons_differ` 1). Re-run anytime before publishing:

```bash
bash scripts/release/test-upgrade-zips.sh
bash scripts/release/test-marker-packs.sh
cargo test --lib registration_server::update::tests::parts
cargo test --lib registration_server::update::tests::upgrade_probe
cargo test --lib registration_server::update::tests::testpath
cargo test --test admin_pages testpath_
(cd desk && cargo test --bin rust-reg-desk probe_ && cargo test --bin rust-reg-desk icons_differ)
```

#### Phase 1 — Publish marker packs — **in progress**

**Actions → Release → Run workflow**, macOS only (`linux=false`, `windows=false`), one run at a time (each pushes a version-bump commit to `main`):

| Run | `version` | `upgrade_probe` | Role | Status |
|---|---|---|---|---|
| 1 | `2.0.0-alpha.9` | *(empty)* | **A** — install pkg | published + verified |
| 2 | `2.0.0-alpha.10` | `B` | **B** — upgrade zips | published + verified (server/wrapper/tray digests ≠ A; Intel job re-run after a CI artifact timeout) |
| 3 | `2.0.0-alpha.11` | `C` | **C** — upgrade zips | after A → B verified |

```bash
gh workflow run release.yml --ref main -f version=2.0.0-alpha.10 -f upgrade_probe=B \
  -f linux=false -f mac=true -f windows=false
```

Per release check: prerelease with pkg + DMG + three zips per mac platform + `SHA256SUMS`; `SHA256SUMS` has no malformed lines; pkg payload has `Library/rust-reg/updates/live` and `relocatable="false"`; B file digests (`rust-reg-macos`, `rust-reg-run-macos`, `rust-reg-desk-macos`) ≠ A.

Note: A (`alpha.9`) was built before `5b3090b8`, so A’s tray has the adopt/kill fixes but **not** the port-file / service Port… change; B and C have both.

#### Phase 2 — Install A on the Mac — **done**

1. Stop the alpha.6 test tray / installed tray (`kill <pid>` — not Quit on an old tray).
2. `sudo bash scripts/release/mac-proof.sh install <rust-reg-2.0.0-alpha.9.pkg>` (pkg, prerelease env, daemon stderr → `/Library/rust-reg/proof.log`).
3. `sudo bash scripts/release/mac-proof.sh port 8081` (after `install`; sets `RUST_REG_HTTP` for A’s wrapper and `data/port` for B/C).
4. Open `/Applications/reg-server.app`; **Port…** → `8081` (A’s tray keeps its own port).
5. Confirm: tray shows `localhost:8081`; no **Upgrade probe**; `GET /testpath` → 404; `proof.log` has no `upgrade-probe-` lines.
6. Record digests:

```bash
PREFIX=/Library/rust-reg
shasum -a 256 "$PREFIX/bin/rust-reg-run" "$PREFIX/bin/rust-reg-desk" \
  "$PREFIX/versions/2.0.0-alpha.9/rust-reg" /Applications/reg-server.app/Contents/MacOS/reg-server
```

**Result (A = alpha.9 installed):** daemon server `versions/2.0.0-alpha.9/rust-reg` on `0.0.0.0:8081`, `/health` ok, `/testpath` 404, 0 `upgrade-probe-` lines in `proof.log`, tray `localhost:8081`. Digests all match A’s `SHA256SUMS`:

| File | sha256 (first 16) |
|---|---|
| `bin/rust-reg-run` | `1b9bc46822744e2d` |
| `bin/rust-reg-desk` | `4aa5b013455a3888` |
| `versions/2.0.0-alpha.9/rust-reg` | `fad870c08c58dd28` |
| `reg-server.app/…/MacOS/reg-server` | `4aa5b013455a3888` |

Gotcha: killing a test tray does not stop the server it spawned — an orphaned rust-reg on the same port gets adopted by the tray while the daemon loops on `Address already in use`. Check the listener’s path (`versions/<A>/rust-reg`, run by root).

#### Phase 3 — Offer B and wait for `current`

`POST /api/update` needs an operator session — start the update from the desk / admin update UI. Status can be read with:

```bash
curl -s "http://127.0.0.1:8081/api/update" | jq .
# poll until phase is current after the restart cycle
```

Then restart wrapper + tray:

```bash
sudo bash scripts/release/mac-proof.sh restart
# Quit tray from menu → reopen /Applications/reg-server.app
```

Confirm every **A → B** row in the [test plan](#test-plan-10). Also live-check the new **Port…** on B’s tray: choose a free port → admin prompt → service moves (`data/port` updated, `/health` on the new port); cancel → stays on 8081.

Optional: publish a B′ with only form changed and same run/desk digests → tray/wrapper **not** re-downloaded (matching digest skip).

**Result (2026-11-09):** A → B started from the events page at 21:44:36. `versions/2.0.0-alpha.10.partial` → `versions/2.0.0-alpha.10` and `updates/live` = `2.0.0-alpha.10` five seconds later; `state.yml` phase `current`, from alpha.9 to alpha.10; A moved to `backup/2.0.0-alpha.9-20261109`.

| Check | Result |
|---|---|
| `upgrade-probe-server-B` in `proof.log` | pass (right after the swap) |
| `upgrade-probe-wrapper-B` in `proof.log` | pass, after `launchctl kickstart -k` (the A wrapper process kept running until then) |
| `/testpath` | pass: 200, probe letter `B` |
| digests: server / wrapper / tray | pass: `f39c949e03ce` / `6aa8fa3de69f` / `b1f7cf60882b`, equal to B |
| tray icon and **Upgrade probe** item after Quit + reopen | pass: icon changed, **Upgrade probe** item present (operator, 2026-11-10) |
| **Port…** on B's tray | pending (operator) |

Found on the way (fixed on main, first shipped in C):

- A fresh install had no `device_id`, so operator login failed with `no_device_id`. The server now writes a UUID v4 at startup (`bae2b18e`). For A, `data/credentials.yml` was written by hand.
- The release check ran once at start, then every six hours. A GitHub 403 (anonymous rate limit, shared VPN exit IP) at start left no offer for hours. A failed check now retries after ten minutes; `GET /api/update` reports `check_failed` and `retry_in`; the events page shows the running version, a **Check for updates** button, and the unreachable message (`160aaa78`, `3a35bb2a`).
- Login: an error stops the countdown and re-enables the button; Enter dismisses the old error and a repeated error flashes again (`f5647f8c`, `c1574acd`).
- A's admin UI was replaced by hand with these builds for review; the A → B swap installed B's own admin.
- Tray **Quit** stops the installed service (`launchctl bootout`, admin prompt). Approving it unloaded the daemon; the reopened tray then started its own `rust-reg-run` as the user against `/Library/rust-reg`, which looped on `credentials.yml: Permission denied`. The daemon was bootstrapped again by hand. Fixed for C (see Findings): Quit leaves the service, no user copy, **Port…** without a prompt.

#### Phase 4 — Offer C

Dispatch C (`alpha.11`, probe `C`). Same check → update → `mac-proof.sh restart` → Quit/reopen tray. Confirm every **B → C** row (probe menu gone; icon ≠ B; logs / `/testpath` = C).

On C also check, with no password prompt anywhere: `ls -l /Library/rust-reg/data/port` shows group `staff`, mode `-rw-rw-r--`; **Port…** → 8082 moves the service (wrapper logs `listen … -> 0.0.0.0:8082; restarting the server`) and back to 8081; Quit closes only the tray and `/health` still answers.

#### Phase 5 — Done

When every row in the test plan passes, set status → `done`. Restore Docker / port settings as needed. Continue to [step 12](#step-12-linux-proof).

### Test plan

| Check | Assert | Result |
|---|---|---|
| A → B tray icon | New mark after Quit + reopen | |
| A → B tray menu | **Upgrade probe** present | |
| A → B wrapper | After service restart, log `upgrade-probe-wrapper-B`; `bin` digest = `versions/<B>/rust-reg-run` | |
| A → B server | Log `upgrade-probe-server-B` | |
| A → B frontend | `/testpath` shows probe B | |
| B → C tray | Probe item gone; icon ≠ B | |
| B → C wrapper / server / frontend | Probe C (or `/testpath` gone) | |
| Matching digest skip | Unchanged component not downloaded; its marker stays | |
| Busy port at tray start | Error “port N is in use by another program; the server did not start”; not adopted | pass (fixed dev tray vs Docker on 8080) |
| **Port…** to a free port | Server moves; other program on the old port untouched | pass (8080 → 8081, Docker kept 8080) |
| **Port…** with installed service (B tray) | Admin prompt; `data/port` written; service on new port | |

### Done when

Every row passes (Phases 0–5). Status → `done`. Continue to [step 12](#step-12-linux-proof).

---

## Step 12. Linux proof

[Back to summary](#summary)

**Status:** not started  
**Depends on:** [11](#step-11-macos-proof) (preferred), [10](#step-10-marker-packs-abc)  
**Unlocks:** [13](#step-13-cleanup)

Prefix: `/opt/rust-reg`. Deb matches `dpkg --print-architecture`. **Required:** tray + wrapper. **Server/frontend:** smoke only (see [Retest server on Linux?](#retest-server-on-linux)).

### Work (manual)

1. Install A. Open **reg-server** tray.
2. Upgrade A → B (tray + wrapper changed; server may skip).
3. `systemctl restart rust-reg` → wrapper probe B. Quit/reopen tray → icon + **Upgrade probe**.
4. Smoke: port listening; `versions/<live>/rust-reg version` is B.
5. Upgrade B → C: probe menu gone; wrapper log C after restart.

### Test plan

| Check | Assert |
|---|---|
| A → B / B → C tray | Icon + **Upgrade probe** appear then disappear |
| A → B / B → C wrapper | Log probe letter after `systemctl restart rust-reg` |
| Server smoke | Socket up; version tag matches (no `/testpath` required) |

### Done when

Table green. Status → `done`. Continue to [step 13](#step-13-cleanup).

---

## Step 13. Cleanup

[Back to summary](#summary)

**Status:** not started  
**Depends on:** [11](#step-11-macos-proof), [12](#step-12-linux-proof)

### Work

1. Remove `/testpath`, **Upgrade probe**, temporary icons, and default-on `upgrade-probe-*` logs from the main line — or keep them only behind `RUST_REG_UPGRADE_PROBE` / debug cfg.
2. Set this plan’s summary statuses for 7–12 as appropriate; mark 13 `done`.
3. Mark release plan step 16 done only when [6](#step-6-automated-parts-suite) and this proof (11–12) are done.

### Test plan

| Scenario | Assert |
|---|---|
| `shipping_build_has_no_probe_menu` | Default desk build: no **Upgrade probe**. |
| `shipping_build_has_no_testpath` | Default form build: no `/testpath` (or gated 404). |
| `parts_still_green` | `cargo test --lib registration_server::update::tests::parts` still passes. |

### Done when

Cleanup scenarios pass. This plan’s steps 1–13 are `done`. Release step 16 can move to `done`; step 17 remains the broader device matrix.

---

## Done when (whole plan)

- Summary statuses 1–13 are `done`.
- [Release step 16](release-implementation-plan.md#step-16-three-zips) automated + observable proof requirements are met.
- Markers are gone or gated; shipping tray/UI unchanged from the product baseline.
