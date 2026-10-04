# Credential file

The venue process keeps `device_id`, the remote base URL, and the project binding in one YAML file. [porting.md](porting.md) step 1 is this file. The wire values are the ones in [login-and-token.md](login-and-token.md). The library is `registration_server::CredentialFile` in `src/registration_server/credentials.rs`.

The file is the whole credential document. A save writes a new document beside it, flushes that document, and only then changes the name. A reboot during the write leaves the previous document in place.

## Document

```yaml
version: 1
device_id: device-1
base_url: https://kuprin.su/
event_id: "5245081"
event_name: TechCrunch
project_token: token-value
```

`version` is `1`. A different version is refused, so a newer file is left untouched.

| field | when it is set |
|---|---|
| `device_id` | provisioning writes it |
| `base_url` | configuration, such as `https://kuprin.su/` or `https://stage.kuprin.su/` |
| `event_id` | the event chosen at bind |
| `event_name` | display label, optional |
| `project_token` | the token from `profile/synchdev` |

A missing file, an empty file, and a file of only whitespace load as an empty document: every field unset, not logged in. YAML `null`, `~`, and `""` are the same as a missing field. A bare number is read as text, so an unquoted event id still loads.

The process is logged in when `event_id` and `project_token` are both non-empty. Logout removes `event_id`, `event_name`, and `project_token`. It leaves `device_id` and `base_url`. One of `event_id` or `project_token` without the other is a damaged document. Load returns an error and does not rewrite the file. A document that still says `expo_id` or `expo_name` loads those as `event_id` and `event_name`. The next save writes the new names.

The file mode is `0600`, subject to the process umask. The token is a secret. `Debug` prints the other fields and the word `set` for the token.

Suggested path: `base/credentials.yml`. The library takes the path from the caller. The name constant is `credentials.yml`.

## Who writes which field

`set_device_id` and `set_base_url` reject an empty or whitespace-only string. `bind` sets `event_id` and `project_token` together, and an optional `event_name`. `clear_binding` removes those three. The in-memory value never holds one of the pair without the other.

Changes go through `CredentialFile::update`. It loads, runs the closure, and stores under one lock. If the closure returns an error, the file is unchanged. The closure must not call `load`, `store`, or `update` on the same file.

`store` replaces the file with a document the caller already holds. `load` then `store` from two tasks can lose one of the edits. `update` is the edit that keeps both the load and the store in one turn.

One process owns the file. Two processes that `update` the same path can still overwrite each other. The bytes a reader sees are always one complete document.

## Commit

The live name and the temporary name are in the same directory, so the rename is atomic on the filesystem.

```text
base/credentials.yml                          previous document, still the live name
base/credentials.yml.tmp-<pid>-<nanos>       new document, mode 0600
        │
        │  write every byte
        │  fsync the temporary file
        ▼
rename temporary → credentials.yml            the name now refers to the new document
        │
        ▼
fsync the directory                            the new name survives power loss
```

Until the rename, readers of `credentials.yml` see the previous document. After the rename they see the new one. They never see a prefix of the new YAML.

A temporary file left by a kill is not read. The next successful save uses a new temporary name. A failed save deletes the temporary file it created.

`store` and `update` return `Ok` after the directory fsync. A power loss after that keeps the new document.

If the rename has not happened, an error leaves the previous document as the live file. If the rename has happened and the directory fsync fails, the new document is already the live file and the error means that name may not be durable yet. The caller stores again.

A document this library cannot parse (invalid YAML, an incomplete binding, a version other than 1) is not replaced. `update` returns the error and leaves the bytes as they were. A file that starts with `---`, `version:`, or `{` is the YAML document. Anything else is the Scala properties import.

The parent directory is created on save if it is missing, then fsynced, including its parent when that directory was just created.

## Scala properties

`load` accepts a Java `Properties` file so a machine that already has `./base/main.props` can be read once. The next `store` writes JSON, under `project_token`, to the path it was given.

| properties key | document field |
|---|---|
| `device_id` | `device_id` |
| `base_url` | `base_url` |
| `current_expo_uid` | `event_id` |
| `current_expo_name` | `event_name` |
| `curentr_expo_token` | `project_token` |
| `project_token` | `project_token` |

The reader follows `Properties.load`: `#` and `!` comments, blank lines, `=`, `:`, or whitespace between key and value, a trailing `\` that joins the next line, and escapes including `\uXXXX`. Other keys are ignored. When a key appears twice, the later line wins.

`load` of `main.props` does not modify it. `store` always rewrites the whole path as the YAML document. Point `store` at `base/credentials.yml`. Copying the credential fields out of `main.props` is a `load` of the old path and a `store` of the new path.
