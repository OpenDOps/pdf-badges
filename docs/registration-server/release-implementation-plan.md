# Release and update — step 11

Build sequence for [porting.md](porting.md) step 11. The desk menu that gains the update control is [registration-ui-implementation-plan.md](registration-ui-implementation-plan.md) step 4. The print channel `print_busy` reads is [design.md](design.md).

Two tracks. The build track compiles a release tag into the binary and publishes the packages. The auto-updater is the registration server and `rust-reg-run`: it reads that compiled tag, asks GitHub if a newer release exists, and swaps the tree after the operator agrees.

The build track is steps 1, 12, and 13. Step 1 is the value the binary carries. Steps 12 and 13 are the pack script and the GitHub Actions run that pass the tag in and upload the assets. The auto-updater is steps 2 through 11. Those steps run on the device. They do not run the workflow. Step 14 is where a release from that workflow is what the updater reads on `https://api.github.com`.

A start that never opens the listen socket is rolled back. A start that does open it keeps the previous tree as a dated backup. The updater installs a release only after the operator agrees and the file matches `SHA256SUMS`.

It does not wait on a printer. Printing's queue, routing, and worker can stay unfinished. `print_busy` is a job already on the print channel, or a print the worker has started. The tests put a job on that channel. They do not start cupsd and do not open a device.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Steps 1 through 11 are `cargo test` and `npm test`. Steps 12, 13, and 14 are manual. `cargo test` does not call `api.github.com`. Step 14 is the one check that uses a release the workflow published: an older binary asks `https://api.github.com` and the desk shows that tag.

Step 11 of [porting.md](porting.md) is done when steps 1 through 14 are done.

## Summary

### Build and GitHub Actions

The version is compiled into the binary, then the workflow packs the selected systems and publishes `release-X.Y.Z`.

| Step | What it covers | Status |
|---|---|---|
| [1. Stamp](#step-1-stamp) | `REGISTRATION_RELEASE_TAG` is baked into the binary and written into the package | done |
| [12. Pack](#step-12-pack) | The deb, the pkg, and the exe from one script, built with that tag | done |
| [13. Publish](#step-13-publish) | `release-X.Y.Z` packs the selected systems and uploads the assets | in progress |

### Auto-updater

The registration server compares the baked tag with a release, then the desk confirms the swap. These steps are tested without a GitHub release. Compare, checksum, unpack, the supervisor, and the socket marker use local files. Look, download, and stage use an HTTP stand-in on `127.0.0.1` that returns the release JSON, the asset, and `SHA256SUMS`. The menu test stubs `fetch`. The production host `https://api.github.com` is step 14.

| Step | What it covers | Status |
|---|---|---|
| [2. Compare](#step-2-compare) | A `vX.Y.Z` tag against the baked release tag | done |
| [3. Look](#step-3-look) | GitHub `releases/latest`, every six hours | done |
| [4. Ask](#step-4-ask) | `GET /api/update` reads the offer. `POST /api/update/check` asks GitHub now | done |
| [5. Menu](#step-5-menu) | "Обновить" when a newer release is waiting | done |
| [6. Checksum](#step-6-checksum) | `SHA256SUMS` against the asset bytes | done |
| [7. Download](#step-7-download) | This machine's asset, after the gate is idle | done |
| [8. Unpack](#step-8-unpack) | `versions/<new>.partial`, then the rename | done |
| [9. Stage](#step-9-stage) | `POST /api/update` writes `staged` and the process exits | done |
| [10. Supervisor](#step-10-supervisor) | The parent reads the phase and starts the next tree | done |
| [11. Started](#step-11-started) | The socket marker, the dated backup, seven days | done |

### Both

Step 14 is the only check that runs the workflow and then the auto-updater against `https://api.github.com`.

| Step | What it covers | Status |
|---|---|---|
| [14. Live](#step-14-live) | A published release is what an older binary offers | not started |

Shared rules:

- The update code lives in `src/registration_server/update.rs`. Tests are `cargo test --lib registration_server::update`. The supervisor binary is `src/bin/rust-reg-run.rs`. It calls the same module. A test of a phase calls the function. It does not have to spawn the binary until step 10.
- The compiled repository is `OpenDOps/pdf-badges`. `REGISTRATION_UPDATE_REPO` overrides it. The value is `owner/repo`. The project token is not sent. The request carries `User-Agent: rust-reg` and `Accept: application/vnd.github+json`, and it has no `Authorization` header.
- A version is `MAJOR.MINOR.PATCH`, optionally followed by `-alpha.N`, `-beta.N`, or `-rc.N`. `N` is `0` or a number that does not start with `0`. A release tag is `v` plus that version. `build.rs` bakes `REGISTRATION_RELEASE_TAG` into the binary the way `SYNC_IN_FLIGHT` is baked. `release_tag()` returns that tag. The auto-updater compares GitHub tags with it. `updates/live`, `state.yml`, and `versions/<version>/` store the version without `v`. The package payload contains a `RELEASE` file with the same tag.
- `state.yml` is `phase`, `from`, `to`, and `rejected`. `phase` is `staged`, `trying`, `current`, or `rolled_back`. `rejected` is the list of versions that failed before the socket opened. `updates/live` is one line, the version to start.
- `updates/live` and `state.yml` are written the way [credentials.md](credentials.md) writes its file: a temporary file in the same directory, fsync, rename, fsync the directory. Readers never see a prefix of the new document.
- `<prefix>` is `/opt/rust-reg` on Linux, `/Library/rust-reg` on macOS, and `C:\Program Files\rust-reg` on Windows. Tests pass a temp directory as the prefix.

```text
<prefix>/
  bin/rust-reg-run
  versions/<version>/
    rust-reg                            rust-reg.exe on Windows
    admin/                              registration-admin dist
    form/                               registration-form build
    RELEASE                             one line, the baked tag
  backup/<version>-<YYYYMMDD>/
  updates/
    live
    state.yml
    started
  data/credentials.yml                  outside versions/ and backup/
  data/<event_id>/                      the event file, same parent as the credential file
```

- The credential file and the event directory stay in `data/`. A swap, a rollback, and a backup move do not rename, delete, or rewrite them. The supervisor passes the same `--credentials <prefix>/data/credentials.yml` and `--dist <prefix>/versions/<live>/admin` on every start.
- The service starts `bin/rust-reg-run` and leaves it running. That process is the parent of the server. It starts `versions/<live>/rust-reg registration-server` and waits. When the child exits and the service has not been asked to stop, the parent reads `state.yml` and starts the next child. A stop of the service stops the child and the parent exits. Version directories are not renamed onto each other.
- The first install may copy `bin/rust-reg-run` and register the service. A later update extracts the version tree and does not replace `bin/rust-reg-run`, the unit, the launchd plist, or the Windows service.
- `GET /updatecheck`, `GET /version`, `GET /update`, and `GET /need_update` stay unserved.
- The gate tests stay green: `cargo test --lib registration_server::gate`.

## Step 1. Stamp

[Back to summary](#summary)

Track: build. The release tag is part of the binary. The pack script and the workflow pass it in. A local build without that variable still has a tag, taken from `Cargo.toml`.

### Work

1. `build.rs` reads `REGISTRATION_RELEASE_TAG`. When it is set, the value is `v` plus the `Cargo.toml` version. Any other value fails the build. The baked string is that tag. When the variable is unset, the baked tag is `v` plus `CARGO_PKG_VERSION`.
2. `release_tag()` returns the baked tag. `rust-reg version` prints that tag and exits 0. `rust-reg-run version` prints the same tag.
3. The version directory and the asset names use the numbers from this tag. The payload includes `RELEASE`, one line, the same tag.
4. This step does not call GitHub and does not pack a deb, pkg, or exe. Steps 12 and 13 pass the tag into this build.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_release_tag_is_baked` | `parse_release_tag("v0.2.0", "0.2.0")` is `v0.2.0`. `release_tag()` equals the tag baked for this build. `rust-reg version` prints that tag. |
| `a_tag_must_match_cargo` | `v0.2.0` with Cargo version `0.1.0` fails. `0.2.0` without `v` fails. An unset variable bakes `v` plus the Cargo version. |

### Done when

`cargo test --lib registration_server::update::tests::stamp` passes. `cargo run -- version` prints the baked tag.

Done. `build.rs` bakes `REGISTRATION_RELEASE_TAG`. Unset, the tag is `v` plus the `Cargo.toml` version. A different value fails the build. `release_tag()` returns that tag. `release_version()` is the numbers used for a version directory and an asset name. `release_file_contents()` is the one line for `RELEASE`. `rust-reg version` and `rust-reg-run version` print the tag. This step does not pack a package.

## Step 2. Compare

[Back to summary](#summary)

Track: auto-updater. A GitHub tag is an offer only when it is a newer version than the tag baked into the binary.

### Work

1. `newer(tag, running)` accepts a tag `v` plus `MAJOR.MINOR.PATCH`, or the same numbers plus `-alpha.N`, `-beta.N`, or `-rc.N`. Comparison is semver order. The three numbers come first. For the same numbers, `alpha` is before `beta`, `beta` is before `rc`, and a version with no suffix is newer than one with a suffix. `running` is `release_tag()` from step 1, compared without the leading `v`.
2. The same version, an older version, a tag without the leading `v`, a fourth numeric component, a suffix other than `alpha.N`, `beta.N`, or `rc.N`, and build metadata (`v1.2.3+sha`) are not newer.
3. This step does not open a socket.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_greater_tag_is_newer` | `v0.2.0` against `0.1.0` is `0.2.0`. `v0.1.1` against `0.1.0` is `0.1.1`. `v1.0.0` against `0.9.9` is `1.0.0`. |
| `the_same_or_older_tag_is_not_offered` | `v0.1.0` and `v0.0.9` against `0.1.0` are not newer. |
| `an_alpha_beta_or_rc_is_ordered` | `alpha.2` is newer than `alpha.1`, `alpha.10` is newer than `alpha.2`, `beta.1` is newer than `alpha.10`, `rc.1` is newer than `beta.2`, and `1.2.3` is newer than `1.2.3-rc.1`. An alpha is not newer than its stable version. |
| `a_tag_that_is_not_a_release_is_rejected` | `0.2.0`, `v0.2`, `v0.2.0.1`, `v1.2.3-b1`, `v1.2.3-alpha`, `v1.2.3-alpha.01`, and `v1.2.3+sha` are not newer. |

### Done when

`cargo test --lib registration_server::update::tests::compare` passes, and step 1 stays green.

Done. `newer` returns the version without `v` when the tag is newer. The order is the three numbers, then `alpha`, `beta`, `rc`, then the pre-release number, and a version with no suffix is newer than the same numbers with a suffix. The same version, an older version, a tag without `v`, a fourth component, any other suffix, and build metadata are not an offer. This step does not open a socket.

## Step 3. Look

[Back to summary](#summary)

Track: auto-updater. A newer release is read from the GitHub repo, `GET /repos/<owner>/<repo>/releases/latest`. The remote registration server has no update path, and this check does not call it. The sync thread only schedules the request: once the listen socket is open, and again every six hours. The call is not a sync download. Nothing is downloaded here.

### Work

1. The request is `GET /repos/<owner>/<repo>/releases/latest` on the configured API base. Tests pass `http://127.0.0.1` and a port. The production base is `https://api.github.com`. `REGISTRATION_UPDATE_REPO=owner/repo` replaces `OpenDOps/pdf-badges`. An empty variable keeps the compiled repository.
2. The request has no project token and no `Authorization` header. A `draft` or `prerelease` body is skipped. `tag_name` is kept when step 2 says it is newer than `release_tag()` and that version is not in `state.yml` `rejected`.
3. A newer tag is stored in memory as the offer. A successful body that is not newer clears the offer. A connect failure, a timeout, or a body that is not the release object leaves the previous offer in place.
4. The first check is due when the listen socket has opened and no check has run. The next scheduled check is due six hours after the last attempt. The sync thread calls it. The call waits on the gate and does not enter the sync queue. The five-second link probe does not call it.
5. The same check function is what `POST /api/update/check` calls in step 4. That call runs even when the six-hour wait has not elapsed. When it returns, the last attempt is now, so the next scheduled check is six hours later.
6. This step does not write `versions/` and does not exit.

### Test scenarios

| Scenario | Assert |
|---|---|
| `latest_is_kept_when_it_is_newer` | The stand-in returns tag `v0.2.0`. The offer is `0.2.0`. The request URL contains `OpenDOps/pdf-badges`. The stand-in saw no `Authorization` header and no token field. |
| `draft_prerelease_and_rejected_are_skipped` | A body with `draft` true, a body with `prerelease` true, and a tag whose version is in `rejected` leave no offer. |
| `a_failed_check_keeps_the_offer` | After an offer is stored, a later `500` leaves that offer. A later `200` whose tag is not newer clears it. |
| `the_repo_override_is_the_path` | `REGISTRATION_UPDATE_REPO=other/name` requests `/repos/other/name/releases/latest`. |
| `the_check_is_due_at_open_and_six_hours` | No previous attempt is due. Five hours after an attempt is not due. Six hours after an attempt is due. |
| `a_forced_check_ignores_the_wait` | Five hours after an attempt, the scheduled check sends nothing. A forced check sends `releases/latest`. The next scheduled check is due six hours after that forced attempt. |

### Done when

`cargo test --lib registration_server::update::tests::look` passes, and steps 1 and 2 stay green.

Done. The sync thread calls `releases/latest` on `https://api.github.com` once the listen socket is open and again six hours after the last attempt. The request sends `User-Agent: rust-reg` and `Accept: application/vnd.github+json`, and it sends no project token. A newer stable tag is the offer. A draft, a prerelease, a tag that is not newer, and a version listed in `updates/state.yml` `rejected` clear the offer. A failed request leaves the previous offer. The call waits until the gate is idle and does not enter the sync queue. `force` runs the same request before the six hours are up. `POST /api/update/check` is step 4.

## Step 4. Ask

[Back to summary](#summary)

Track: auto-updater. The desk can read the offer. `POST /api/update/check` runs the step 3 GitHub lookup immediately. Neither route downloads a file or exits.

### Work

1. `GET /api/update` requires the `desk` cookie already used by the desk routes. A missing cookie is `401` `unauthorized`.
2. Success is `{ "ok": true, "data": { "version", "newer" } }`. `version` is `release_tag()` from step 1 with the leading `v` removed. `newer` is the offer from step 3, or `null` when there is none.
3. `GET /api/update` does not open a connection to the release host, does not write `updates/`, and does not exit the process.
4. `POST /api/update/check` requires the same desk cookie. It calls the step 3 check now, without waiting out the six hours. The response is the same success body, after that check returns. A connect failure, a timeout, or a body that is not the release object is `502` `update_check_failed`. The stored offer stays as step 3 left it. A check already in flight is `429` `check_in_progress` and does not open a second request.
5. `GET /updatecheck`, `GET /version`, `GET /update`, and `GET /need_update` stay `404`. The menu in step 5 does not gain a control that posts this route.

### Test scenarios

| Scenario | Assert |
|---|---|
| `update_names_the_newer_version` | With the desk cookie and an offer of `0.2.0`, the data is `version` `0.1.0` and `newer` `0.2.0`. The stand-in sees no second request. `updates/` is absent. |
| `update_without_an_offer` | With the cookie and no offer, `newer` is `null` and `version` is the running version. |
| `update_requires_the_desk_cookie` | No cookie is `401` `unauthorized` for both `GET /api/update` and `POST /api/update/check`. |
| `check_asks_github_now` | Five hours after a check, `POST /api/update/check` with the desk cookie sends one `releases/latest`. The data `newer` is the tag that body names. `updates/` is absent. The process stays up. |
| `check_failure_keeps_the_offer` | The stand-in returns `500`. The response is `502` `update_check_failed`. A following `GET /api/update` still has the previous `newer`. |
| `a_second_check_waits` | A check still in flight answers the second `POST /api/update/check` with `429` `check_in_progress`. The stand-in has seen one request. |
| `the_scala_update_paths_stay_unserved` | `GET /updatecheck`, `GET /version`, `GET /update`, and `GET /need_update` are `404`. |

### Done when

`cargo test --lib registration_server::update::tests::ask` passes, and steps 1 through 3 stay green.

Done. `GET /api/update` with the desk cookie returns the running version and the stored offer, or `null` when there is none. It does not call the release host. `POST /api/update/check` runs that check immediately and returns the same body. A failed check is `502` `update_check_failed` and the previous offer stays. A check already in flight is `429` `check_in_progress`. A missing cookie is `401` `unauthorized`. `GET /updatecheck`, `GET /version`, `GET /update`, and `GET /need_update` stay `404`.

## Step 5. Menu

[Back to summary](#summary)

Track: auto-updater. The desk menu shows "Обновить" when step 4 returns a newer version. Confirming is the only way to start the install.

### Work

1. The menu loader reads `GET /api/update`. When `newer` is a version, the screen shows a button "Обновить" and that version. The click posts `POST /api/update` and sends credentials.
2. When `newer` is `null`, or the request fails, the button is absent.
3. The moderation checkbox stays absent. `/register` does not render the button. The screen still has no list of local addresses.
4. `menu_omits_update_and_moderation` stays the case with no newer version. The waiting-update case is a new test in the same file.

### Test scenarios

| Scenario | Assert |
|---|---|
| `menu_omits_update_and_moderation` | With `newer` null, there is no "Обновить" button and no "Включить модерацию" checkbox. |
| `menu_offers_the_update` | With `newer` `0.2.0`, "Обновить" is shown and the version is on the screen. The click posts `POST /api/update`. No release-host URL is fetched. |
| `register_omits_the_update` | `/register` with the same offer has no "Обновить" button. |

### Done when

`npm test -- menu_` in `registration-form/` passes, and steps 1 through 4 stay green.

Done. The menu loader reads `GET /api/update`. A `newer` version shows the button "Обновить" and that version. The click posts `POST /api/update` with credentials. A null offer, or a failed read, leaves the button off. The moderation checkbox stays absent. `/register` does not show the button. The menu does not post `POST /api/update/check` and does not fetch a release host.

## Step 6. Checksum

[Back to summary](#summary)

Track: auto-updater. The asset is installed only when its digest is the line `SHA256SUMS` names.

### Work

1. `SHA256SUMS` is one line per file: lowercase hex, two spaces, the file name. A line with one space, or a name that is not on the list, is `checksum_mismatch`.
2. `verify(bytes, sums, name)` compares the SHA-256 of `bytes` with the line for `name`.
3. A mismatch deletes the temporary asset file. `updates/live` is left unchanged. This step does not unpack.

### Test scenarios

| Scenario | Assert |
|---|---|
| `the_digest_matches_the_named_line` | Bytes whose digest is the line for `rust-reg-0.2.0.pkg` verify. A second line for another file is ignored. |
| `a_mismatch_deletes_the_file` | The asset file is removed. The error is `checksum_mismatch`. A sibling `updates/live` still reads the previous version. |
| `a_broken_sums_line_is_a_mismatch` | One space between the digest and the name, or a name that is absent, is `checksum_mismatch`. |

### Done when

`cargo test --lib registration_server::update::tests::checksum` passes, and steps 1 through 5 stay green.

Done. `verify` accepts a line of lowercase hex, two spaces, and the asset name. A second line for another file is ignored. One space before the name, a different digest, or a missing name is `checksum_mismatch`. `verify_asset` deletes that temporary file on a mismatch and does not change `updates/live`. This step does not unpack.

## Step 7. Download

[Back to summary](#summary)

Track: auto-updater. The machine downloads its own asset. The download waits until a local request and a print job already in progress have finished.

### Work

1. The asset name for the offer `X.Y.Z` is `rust-reg_X.Y.Z_amd64.deb` on Linux, `rust-reg-X.Y.Z.pkg` on macOS, and `rust-reg-X.Y.Z.exe` on Windows. The same names are the ones step 12 packs and step 13 uploads.
2. The file is written next to `versions/` under a temporary name. `SHA256SUMS` is downloaded from the same release and checked with step 6. A mismatch deletes the temporary file and returns `checksum_mismatch`.
3. Before the first byte, the download waits until the gate has no print in progress and no local request other than this one. The update request drops its own guard for that wait and takes it again when the download returns. Between chunks the task yields, so the accept loop can take a request that arrived during the download.
4. The stand-in records the paths. This step does not rename a directory under `versions/` and does not exit.

### Test scenarios

| Scenario | Assert |
|---|---|
| `the_asset_for_this_os_is_downloaded` | The stand-in sees the asset name for the current OS and `SHA256SUMS`. The temporary file's digest matches. `versions/` has no new directory. |
| `the_download_waits_out_a_print` | `enter_print` is held. The stand-in sees no request. After the guard drops, the asset and `SHA256SUMS` are fetched. |
| `a_bad_digest_leaves_no_file` | `SHA256SUMS` names a different digest. The temporary file is gone. `updates/live` is unchanged. The error is `checksum_mismatch`. |

### Done when

`cargo test --lib registration_server::update::tests::download` passes, `cargo test --lib registration_server::gate` stays green, and steps 1 through 6 stay green.

Done. The asset name is `rust-reg_X.Y.Z_amd64.deb` on Linux, `rust-reg-X.Y.Z.pkg` on macOS, and `rust-reg-X.Y.Z.exe` on Windows. It is written next to `versions/` as `<name>.partial`. `SHA256SUMS` from the same release is checked with step 6. A mismatch deletes that file and returns `checksum_mismatch`. The download waits until the gate is idle, dropping the caller's request guard for the wait and taking it again when the call returns. `versions/` gains no directory. The process does not exit.

## Step 8. Unpack

[Back to summary](#summary)

Track: auto-updater. The checked asset becomes a complete version directory at the rename, and not before.

### Work

1. The version payload inside the asset is `rust-reg` (`rust-reg.exe` on Windows), `admin/`, `form/`, and `RELEASE`. Extract that payload into `versions/<new>.partial`. Fsync the files. Rename the directory to `versions/<new>/`. The rename is the moment the tree is complete.
2. Linux extraction is `dpkg-deb -x` into a temporary directory, then the version subtree is what moves to `.partial`. macOS extraction expands the pkg without `installer`. Windows runs the exe with a staging directory and without the service path. Step 12 builds those assets. This step's automated test feeds a payload directory that already has those entries, which is the directory those tools produce.
3. A `.partial` directory left behind is not a version the supervisor will start. `bin/rust-reg-run` is not replaced. `data/credentials.yml` and `data/<event_id>/` are not moved.
4. This step does not write `state.yml` and does not exit.

### Test scenarios

| Scenario | Assert |
|---|---|
| `the_rename_publishes_the_tree` | After the call, `versions/0.2.0/` contains `rust-reg`, `admin/`, `form/`, and `RELEASE`. `RELEASE` is the line `v0.2.0`. `versions/0.2.0.partial` is gone. |
| `a_failed_extract_leaves_the_partial` | The extract returns an error before the rename. `versions/0.2.0/` is absent. The `.partial` directory is removed. `updates/live` still names `0.1.0`. |
| `data_stays_put` | `data/credentials.yml` and `data/EVT/db.sqlite` have the same bytes after a successful unpack. |

### Done when

`cargo test --lib registration_server::update::tests::unpack` passes, and steps 1 through 7 stay green.

Done. `unpack` copies `rust-reg` (`rust-reg.exe` on Windows), `admin/`, `form/`, and `RELEASE` into `versions/<new>.partial`, fsyncs, and renames that directory to `versions/<new>/`. The test feeds the directory those extract tools produce. A failed copy removes `.partial` and does not create `versions/<new>/`. `updates/live`, `data/credentials.yml`, `data/<event_id>/`, and `bin/rust-reg-run` stay put. This step does not write `state.yml` and does not exit.

## Step 9. Stage

[Back to summary](#summary)

Track: auto-updater. `POST /api/update` is the operator's confirmation. It downloads, checks, unpacks, records `staged`, and exits. A print still in progress refuses the request.

### Work

1. `POST /api/update` requires the desk cookie. A missing cookie is `401` `unauthorized`.
2. `print_busy` is true when the print channel has a job, or the gate's print flag is set. The handler's own request guard does not count. The response is `409` `print_busy`. Nothing is downloaded, `updates/` is unchanged, and the process stays up. The test enqueues one job and does not start a worker, then repeats with `enter_print` held and an empty channel.
3. Otherwise run steps 7 and 8. Write `state.yml` with phase `staged`, `from` the running version, and `to` the offer. Delete `updates/started` when it is present. Fsync the state file. `updates/live` still names `from`. `from` is `release_tag()` with the leading `v` removed.
4. Return `200` and `{ "ok": true, "data": { "version": "<to>" } }`. After that response is sent, exit the process. The test replaces the exit with a recorded flag so the test process stays up. A `checksum_mismatch` from step 7 is `409` `checksum_mismatch` and does not exit.

### Test scenarios

| Scenario | Assert |
|---|---|
| `a_queued_job_is_print_busy` | One job is on the channel. The response is `409` `print_busy`. The stand-in sees no download. The exit flag is clear. |
| `a_running_print_is_print_busy` | The channel is empty and `enter_print` is held. The response is `409` `print_busy`. The exit flag is clear. |
| `stage_keeps_live_and_exits` | With an offer and a matching digest, `state.yml` is `staged`, `from` `0.1.0`, `to` `0.2.0`. `updates/live` is `0.1.0`. `updates/started` is absent. `versions/0.2.0/rust-reg` exists. The exit flag is set. `data/credentials.yml` is unchanged. |

### Done when

`cargo test --lib registration_server::update::tests::stage` passes, and steps 1 through 8 stay green.

Done. `POST /api/update` requires the desk cookie. A job on the print channel, or the gate's print flag, is `409` `print_busy`: nothing is downloaded and the process stays up. Otherwise the offer's asset is downloaded and checked, then unpacked into `versions/<to>/`. `state.yml` is `staged` with `from` the running version and `to` the offer. `updates/started` is removed. `updates/live` still names `from`. The response is `200` and `{ "version": "<to>" }`, then the process exit is recorded. A bad digest is `409` `checksum_mismatch` and does not exit.

## Step 10. Supervisor

[Back to summary](#summary)

Track: auto-updater. `bin/rust-reg-run` is the process the service starts. It stays the parent, reads the phase when a child exits, and starts the version in `updates/live`.

### Work

1. Add the `rust-reg-run` binary. It takes `--prefix`. `resume(prefix)` is the decision, and the binary calls it. The child is a path the test supplies, so the test does not start the registration server.
2. Phase `staged`: write `updates/live` as `to`, fsync, set the phase to `trying`, fsync `state.yml`, then start `versions/<to>`. The phase is durable before the start.
3. Phase `trying` with no `updates/started`: write `updates/live` back to `from`, delete `versions/<to>/`, append `to` to `rejected`, set the phase to `rolled_back`, and start `versions/<from>`. `versions/<from>/` was never moved.
4. Phase `trying` with `updates/started`: start `versions/<to>` and leave that directory in place. The backup move and the write of phase `current` are step 11. This branch calls that move once step 11 has it.
5. Phase `current` or `rolled_back`: start the version in `updates/live`.
6. When the service asks the parent to stop, the parent stops the child and exits. That stop does not change the phase and does not delete a version directory.
7. A version in `rejected` is the one step 3 will not offer again.

### Test scenarios

| Scenario | Assert |
|---|---|
| `staged_points_live_at_the_new_tree` | Starting from `staged` writes `live` as `0.2.0`, sets `trying`, and starts `versions/0.2.0`. The phase file is `trying` before the child is started. |
| `trying_without_started_rolls_back` | No `started` file. `live` returns to `0.1.0`. `versions/0.2.0/` is gone. `versions/0.1.0/` remains. The phase is `rolled_back` and `rejected` contains `0.2.0`. The child started is `0.1.0`. |
| `trying_with_started_keeps_the_new_tree` | `updates/started` is present. The child started is `versions/0.2.0`. That directory is still on disk. |
| `current_starts_live` | Phase `current` and `live` `0.2.0` starts that version. `state.yml` is unchanged. |
| `a_service_stop_leaves_the_phase` | A stop while the child is running exits the parent. The phase and both version directories stay. |

### Done when

`cargo test --lib registration_server::update::tests::supervisor` passes, and steps 1 through 9 stay green. `cargo run --bin rust-reg-run -- --prefix <temp>` on a `staged` fixture performs the same live-file write as `staged_points_live_at_the_new_tree`.

Done. `resume` reads the phase. `staged` writes `updates/live` as `to`, sets `trying`, and only then starts `versions/<to>`. `trying` without `updates/started` points `live` back at `from`, deletes `versions/<to>/`, appends `to` to `rejected`, sets `rolled_back`, and starts `versions/<from>`. `trying` with `started` finishes the backup move and starts `versions/<to>`, which stays on disk. `current` starts the version in `updates/live` and leaves `state.yml` unchanged. A stop while the child is running exits the parent and leaves the phase and both version directories. `rust-reg-run --prefix` calls that same decision.

## Step 11. Started

[Back to summary](#summary)

Track: auto-updater. The new process marks the socket open, then the previous tree becomes a backup. A backup older than seven days is removed.

### Work

1. After the listen socket is open, and before the sync thread's first update check, write `updates/started` and fsync it. The ready signal in `registration_server::run` is that moment.
2. While the phase is `trying`, move `versions/<from>/` to `backup/<from>-<YYYYMMDD>/`, set the phase to `current`, and fsync `state.yml`. The date is the local date. A test passes the date.
3. Remove each backup directory whose date is more than seven days before that date. A backup from the seventh day stays. The move does not follow a child that is still running. Step 10's `trying` plus `started` path calls this same move when the child exited before it finished.
4. A later start with phase `current` does not move `versions/` again and does not rewrite `started`.
5. `data/credentials.yml` and the event directory stay in place across the move and the cleanup.

### Test scenarios

| Scenario | Assert |
|---|---|
| `the_open_socket_writes_started` | A run that binds the listen socket creates `updates/started`. A run that fails before the bind leaves it absent. |
| `trying_moves_the_old_tree` | Phase `trying`, `from` `0.1.0`, date `2026-10-06`. `versions/0.1.0/` becomes `backup/0.1.0-20261006/`. The phase is `current`. `versions/0.2.0/` stays. |
| `a_backup_older_than_seven_days_is_removed` | On `2026-10-06`, `backup/0.1.0-20260929/` is removed. `backup/0.1.0-20260930/` stays. |
| `current_does_not_move_again` | A second start with phase `current` leaves `versions/0.2.0/` and `backup/0.1.0-20261006/` where they are. `data/credentials.yml` is unchanged. |

### Done when

`cargo test --lib registration_server::update::tests::started` passes, and steps 1 through 10 stay green.

Done. After the listen socket binds, `updates/started` is written and fsynced, before the sync thread starts. A bind that fails leaves it absent. While the phase is `trying`, `versions/<from>/` moves to `backup/<from>-<YYYYMMDD>/`, the phase becomes `current`, and a backup dated seven or more days earlier is removed. The date is the local date; a test passes the date. A later start with phase `current` does not move `versions/` and does not rewrite `started`. `data/credentials.yml` stays put.

## Step 12. Pack

[Back to summary](#summary)

Track: build. One script builds the asset for the OS it is run on and passes the release tag into step 1. The first install registers the service. A later update uses the extract path from step 8 and leaves the supervisor in place.

### Work

1. `scripts/release/pack.sh` reads `REGISTRATION_RELEASE_TAG`. When it is unset, the script sets it to `v` plus the `Cargo.toml` version. `cargo build --release` runs with that variable set, so the binary bakes the tag. The script also runs `npm run build` in `registration-admin/` and `registration-form/`. The payload is `rust-reg` (`rust-reg.exe` on Windows), `admin/` from `registration-admin/dist`, `form/` from `registration-form/build`, and `RELEASE` containing the tag.
2. On Linux the asset is a deb built with `dpkg-deb`. The version subtree lives under `opt/rust-reg/versions/<version>/`. `opt/rust-reg/bin/rust-reg-run` and the systemd unit are in the archive. `DEBIAN/postinst` enables the unit. The unit starts `<prefix>/bin/rust-reg-run` and restarts it when the process exits. An operator stop of the unit does not start it again.
3. On macOS the asset is a pkg built with `pkgbuild`. The payload is the same tree under `/Library/rust-reg`. A launchd plist starts the supervisor with `KeepAlive`. The update path expands the pkg with `pkgutil` and does not run `installer`.
4. On Windows the asset is `rust-reg-<version>.exe`. Run with a staging directory and without the service flag, it writes the payload and does not register a service. The first-install run registers the service that starts `bin/rust-reg-run` and restarts it when the process exits.
5. The script prints a `SHA256SUMS` line for the asset it wrote: lowercase hex, two spaces, the file name from step 7.
6. Replacing an existing install with the update path does not change the bytes of `bin/rust-reg-run`.

### Check

Run this on the OS you have. The workflow in step 13 runs it on the other two.

| Check | Result |
|---|---|
| `scripts/release/pack.sh` | The asset name matches step 7 for this OS. `shasum -a 256 -c` accepts the printed `SHA256SUMS` line. The payload contains `rust-reg`, `admin/`, `form/`, and `RELEASE`. `rust-reg version` prints the tag in `RELEASE`. |
| First install into a scratch prefix | `bin/rust-reg-run` exists. The service definition starts that binary. `versions/<version>/` has the payload. `data/` is empty of a version tree. |
| Update extract of a second version | `versions/<new>/` has the new payload. The bytes of `bin/rust-reg-run` are the bytes from the first install. The service definition is unchanged. `versions/<new>/rust-reg version` prints the new tag. |

### Done when

The three checks have been run on this machine, and steps 1 through 11 stay green.

Done. `scripts/release/pack.sh` builds the asset for the OS it is run on. Unset, `REGISTRATION_RELEASE_TAG` is `v` plus the `Cargo.toml` version, and that value is what `cargo build --release` bakes. The payload is `rust-reg` (`rust-reg.exe` on Windows), `admin/` from `registration-admin/dist`, `form/` from `registration-form/build`, and `RELEASE`. On this Mac the asset is `rust-reg-0.1.0.pkg`. `shasum -a 256 -c` accepts the printed line. `versions/0.1.0/rust-reg version` prints `v0.1.0`, the line in `RELEASE`. A scratch install has `bin/rust-reg-run`, a launchd plist that starts that binary with `KeepAlive`, and no version tree under `data/`. Expanding the pkg with `pkgutil` and copying only `versions/0.1.0/` leaves `bin/rust-reg-run` and the plist byte for byte.

## Step 13. Publish

[Back to summary](#summary)

Track: build. A tag `release-X.Y.Z`, or a manual run of that same `X.Y.Z`, packs the selected systems and publishes the GitHub release `release-X.Y.Z`. The workflow writes those numbers into `Cargo.toml` and pushes the commit when the file changed. The binary bakes `vX.Y.Z`.

### Work

1. `.github/workflows/release.yml` runs on a push of a tag `release-*`, and on a manual run. The manual inputs are checkboxes for Linux, macOS, and Windows, each checked by default, and a version. A version is `x.x.x`, `x.x.x-alpha.N`, `x.x.x-beta.N`, or `x.x.x-rc.N`. A tag push builds all three. The first job fails unless the version has that form and at least one system is selected. It sets the `[package]` version in `Cargo.toml`, and the `rust-reg` version in `Cargo.lock`, to that version. When either file changed, it commits and pushes: a manual run pushes to the branch it was started from, and a tag pushes to the default branch. The pack jobs build that commit.
2. The selected systems run as a matrix: `ubuntu-latest`, `macos-latest`, and `windows-latest`. Each sets `REGISTRATION_RELEASE_TAG` to `v` plus that version and runs `bash scripts/release/pack.sh`. Each job uploads its asset.
3. One later job writes `SHA256SUMS` from the lines those jobs printed and creates or updates the release `release-X.Y.Z`. The release carries the assets that were built and `SHA256SUMS`. Uploading again replaces a file of the same name and leaves assets from systems that were not selected. A draft is not used. A version with `-alpha`, `-beta`, or `-rc` is marked pre-release. A version without that suffix is not.
4. The workflow does not deploy to a device. Installing remains the auto-updater, step 9, on the machine.

### Check

Push a tag `release-` plus the `Cargo.toml` version, or run the workflow by hand with that version. This publishes a real release. Delete that release afterwards if the tag was only a trial.

| Check | Result |
|---|---|
| The tag `release-` plus the `Cargo.toml` version | The Actions run is green. The release `release-X.Y.Z` has the three asset names from step 7 for that version, and `SHA256SUMS`. |
| A manual run with one system unchecked | That system is not built. The release gains or replaces only the assets that were built. |
| `shasum -a 256 -c SHA256SUMS` | Every line matches the downloaded file it names. |
| A version that is not `x.x.x`, `x.x.x-alpha.N`, `x.x.x-beta.N`, or `x.x.x-rc.N` | The workflow fails before packing. No release is published. |
| A version that differs from `Cargo.toml` | The workflow commits that version and pushes it, then packs that commit. |
| `Cargo.toml` already at that version | No version commit is pushed. |
| The packed binary | `rust-reg version` prints `v` plus the `Cargo.toml` version. `RELEASE` in the payload is that same line. |

### Done when

These checks have been run, and steps 1 through 12 stay green.

## Step 14. Live

[Back to summary](#summary)

Track: both. The workflow has published a release, and a binary baked with an older tag reads that release from `https://api.github.com`. The stand-in is not running. This check uses a scratch prefix.

### Work

1. Two tags, both built by the step 13 workflow. Tag A is the older one. Tag B is newer, and it is the latest release on `OpenDOps/pdf-badges`. The binary under test is the tag A build: `rust-reg version` prints `v` plus A.
2. Start that binary as the registration server against a scratch prefix. Leave the update API base on `https://api.github.com`. `REGISTRATION_UPDATE_REPO` is unset, or set to `OpenDOps/pdf-badges`.
3. After the listen socket is open, `GET /api/update` with the desk cookie returns `version` A and `newer` B. `gh api repos/OpenDOps/pdf-badges/releases/latest` returns the tag `v` plus B.
4. `POST /api/update` downloads this machine's asset from that release. The file matches the release's `SHA256SUMS`. `state.yml` is `staged`, `from` A, `to` B. `updates/live` still names A. The process then exits.
5. `rust-reg-run --prefix` on that scratch prefix writes `updates/live` as B and starts `versions/B`.

### Check

| Check | Result |
|---|---|
| `gh api repos/OpenDOps/pdf-badges/releases/latest` | `tag_name` is `v` plus B. The step 13 run for that tag is green. |
| `GET /api/update` on the tag A binary | `version` is A and `newer` is B. The request host is `api.github.com`. |
| `POST /api/update` | The asset digest matches `SHA256SUMS` on the release. `state.yml` is `staged` from A to B. `data/credentials.yml` is unchanged. |
| `rust-reg-run --prefix` the scratch prefix | `updates/live` is B. The child that starts is `versions/B/rust-reg`, and `rust-reg version` prints `v` plus B. |

### Done when

The four checks have been run, and steps 1 through 13 stay green.
