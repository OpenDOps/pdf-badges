# Event database — step 4

Build sequence for [porting.md](porting.md) step 4. The schema and the reason for the memory copy are [db-design.md](db-design.md).

This plan opens the event file, loads the list into memory, and answers search, the list page, a visitor write, and the stored JSON. It does not download visitors, bind a project, or edit operator keys. The HTTP routes that call these functions come later.

Each step is one change. The tests named in the step land with it and stay green. A later step keeps the earlier tests passing. Set **Status** to `not started`, `in progress`, or `done`.

Step 4 is done when steps 1 through 6 are done.

## Summary

| Step | What it covers | Status |
|---|---|---|
| [1. Open](#step-1-open) | File schema, memory schema, empty database is ready | done |
| [2. Load](#step-2-load) | Background copy, first commit of 100, `index_state` | done |
| [3. Search](#step-3-search) | Name, surname, company, category, `last_add`, `last_print` | done |
| [4. Write](#step-4-write) | File first, then the memory row and the index; a print updates order | done |
| [5. Table](#step-5-table) | One page of the list | done |
| [6. Full row](#step-6-full-row) | The JSON stored on the file | done |

Shared rules:

- The library is `registration_server::event_db` in `src/registration_server/event_db.rs`. `rusqlite` is `bundled`, so FTS5 is in the binary.
- Tests are `cargo test --lib registration_server::event_db`. Each test uses its own temp directory. They do not call kuprin.su and they do not start the HTTP server.
- `open` takes the directory and the event id. The file is `base/{event_id}/db.sqlite` when the directory is `base`. Tests pass the temp directory.
- Step 1 creates a new file. Steps 2 through 6 copy the Scala event file into that temp directory and open the copy. They do not write the original.

```text
/Users/mac/Documents/development/JJO/Irbis/temp-reg/registration_x86/base/GELCPAQSFL/db.sqlite
```

The copy is 5,176 visitors and 765 print rows (706 distinct uids). Keys are `admin` (admin) and `IPTB2PA`. `u_fulltext` names are lowercase. `gotsome` is blank and loads as `0`. `give_packet` is `0`. Rows the tests name:

| uid | list name | what it shows |
|---|---|---|
| 733 | илья агранович, category 7 | newest `mem_u.ts` (`1790827532986`), never printed. JSON `personalData.name` is `Илья` |
| 249 | андрей смушкин | latest print, `prints.ts` `1790784637258`, one print |
| 3534 | | 20th by `last_add` |
| 3535 | т эмирова | 21st by `last_add`, so not on the first page |
| 732 | альфия тимербаева | email `alfa23@inbox.ru` |
| 1 | татьяна юрьевна кирилловская | company `BEAUTYDRUGS` |

Category 7 has 2,088 visitors. `open` on this copy adds the four `mem_u` indexes when they are missing. It does not add `prints_unique`, does not change `man_to_user.uid`, and does not drop `prints.category`, `prints.is_cert`, or `mem_u.new_id`.

- `write` stores the JSON string it is given. It does not parse a remote visitor body. The caller passes the list columns. The sync slice is what will extract those columns.
- Search reads the memory database. It does not take an indexing flag. It returns the rows committed so far. The list page reads the file until the copy is `ready`, then reads memory.
- The file has no FTS virtual table.

## Columns

`list_row` is a normal table in the memory database. `u_fts` is FTS5 with `content='list_row'` and only the three text columns. Category and the two orders are btree indexes on `list_row`. An FTS5 table cannot carry those indexes, so putting `category` on the FTS row would make a category filter read every visitor.

| column | indexed | source |
|---|---|---|
| `name`, `surname`, `c_name` | FTS5 tokens, `unicode61` | `u_fulltext`, copied as stored. This fixture is lowercase. Original case is in `mem_u.data` |
| `category` | btree | `mem_u.category`. A filter is `IN` of integers |
| `added_ts` | btree | `mem_u.ts`. Order `last_add` |
| `last_print_ts` | btree | `MAX(prints.ts)`, else `0`. Order `last_print` |
| `print_count` | no | count of `prints` rows. The Scala name `prints_unique.ts` is this count |
| `uid`, `ticket_status`, `gotsome`, `give_packet`, `org_id`, `email` | no | the list page |
| `mem_u.data` | file only | the full JSON |

`last_add` is `ORDER BY added_ts DESC, uid`. `last_print` is `ORDER BY last_print_ts DESC, added_ts DESC, uid`. A visitor who has never been printed has `last_print_ts` `0` and sorts last.

The Scala search orders only by `mem_u.ts`. Its unfiltered page is `db_view_buf`, a list that prepends a uid on print or on add and is dropped on restart. These two orders replace that list. Both timestamps are on the file, so a restart keeps them.

A search token is one word. The word is an FTS5 phrase: a `"` inside it is doubled, the word is wrapped in `"`, and a `*` is appended outside the quotes. The finished `MATCH` text is a bound parameter, so the word is not concatenated into the SQL. A word with no letter or digit is not sent to FTS, so `*`, `"`, and the operators `AND`, `OR`, `NOT`, and `NEAR` cannot change the query or raise a syntax error. Several words are AND of (`name:` OR `surname:` OR `c_name:`), which is the Scala `MATCH`. A category number is not a token: matching the digits of a category finds nothing unless those digits are in a name. Search always uses the prefix `*`.

## Step 1. Open

[Back to summary](#summary)

Create both databases and their schema. An empty event is `ready`.

### Work

1. Add `rusqlite` with `bundled`.
2. `open` creates the file if it is missing, turns WAL on, and sets `synchronous=FULL`.
3. The new file matches the Scala event file. `mem_u` includes `data`, `ts`, and `new_id`. `u_fulltext` is an ordinary table. `prints` is `uid`, `ts`, `category`, `is_cert`. There is no `prints_unique` table. `man_to_user.uid` is `INTEGER`. The other tables from [db-design.md](db-design.md) are created in the same open. Indexes on `mem_u(code)`, `mem_u(barcode)`, `mem_u(repl_barcode)`, and `mem_u(in_synch)` are created when missing. `keys` gets one admin row and one operator row only when the table is empty.
4. The memory database is `file:{name}?mode=memory&cache=shared`. The name is unique to that open, so two events do not share pages. Create `list_row`, the three btree indexes, and `u_fts`.
5. `index_state` is `ready` when the file has no visitors. A file that already has visitors stays `loading` until step 2 copies them. `loaded_count` is `0`.
6. A new file is written as `db.sqlite.creating` and renamed to `db.sqlite` after the schema commits. A live file that is not a database, fails `quick_check`, is missing an event table, or contains a virtual table is moved, with its `-wal`, `-shm`, and `-journal` files, to `{event_id}/broken/{index}-{unix_millis}/`. A file that does not start with a SQLite header is moved without being opened. A file that does has its `-wal`, `-shm`, and `-journal` copied first, because opening it truncates them. The index is one higher than any backup already there. The time is the drop time in unix milliseconds. The original file names stay inside that directory. An empty file is then created at `db.sqlite`. A leftover `db.sqlite.creating` with no live file is archived the same way. A healthy live file is kept, including its rows and its `keys`. A leftover creating file beside a healthy live file is archived and does not mark a reload.
7. When a database file is archived, that event id is appended to `full_reload` in `{directory}/base.config`. A first create does not write the file. `open` does not remove an id.

```yaml
version: 1
full_reload:
  - GELCPAQSFL
```

8. `switch` opens the next event before it drops the previous connections. If the next file must be recreated and `base.config` cannot be written, the previous event stays open and the broken file stays on disk.

### Test scenarios

| Scenario | Assert |
|---|---|
| `open_creates_file` | After `open` on an empty directory, `db.sqlite` exists, `mem_u` has no rows, and `index_state` is `ready`. |
| `file_matches_scala` | `mem_u` has `data`, `ts`, and `new_id`. `prints` has `category` and `is_cert`. `man_to_user.uid` is integer. There is no `prints_unique` table and no `VIRTUAL TABLE`. |
| `memory_indexes` | `list_row` has indexes on `category`, `added_ts`, and `last_print_ts`. The `u_fts` SQL names `name`, `surname`, and `c_name`, and does not name `category`. |
| `empty_keys` | `keys` has two rows, one admin and one operator. |
| `second_open_keeps_keys` | Opening the same file again does not insert new keys and does not write `base.config`. |
| `adds_missing_indexes_and_keeps_rows` | A file with the Scala tables, one visitor, and one key gains the four `mem_u` indexes. The row and the key stay. `index_state` is `loading`. `base.config` is not written. |
| `adds_new_id_without_dropping_rows` | A `mem_u` without `new_id` gains the column. The visitor stays. `base.config` is not written. |
| `switch_keeps_the_previous_file` | After `switch` to another event, the previous file still has its visitor and its keys. The new memory list is empty. `base.config` is not written. |
| `two_events_do_not_share_memory` | A row inserted in one open is absent from the other open. |
| `switch_to_the_same_event_is_a_noop` | `switch` to the current id keeps the memory rows. |
| `rejects_an_event_id_that_is_not_one_path_component` | An empty id, `..`, or an id with a slash is refused. |
| `broken_file_is_restored_and_marked` | Garbage bytes in `db.sqlite`, plus `-wal` and `-shm`, are moved to `broken/1-{millis}/` and an empty schema replaces them. `quick_check` is `ok`. `base.config` lists that event id. |
| `corrupt_sqlite_header_keeps_sidecar_bytes` | A file with a SQLite header and a torn body is archived with the `-wal` and `-shm` bytes from before SQLite opened it. |
| `truncated_header_is_restored_and_marked` | A file that starts with the SQLite header and is then cut short is archived with index 1, and the id is listed. |
| `missing_table_is_restored_and_marked` | A SQLite file that only has an unrelated table is archived. The new file does not have that table. The archived file still does. The id is listed. |
| `reload_mark_lists_the_event_once` | Restoring the same event twice leaves one entry in `full_reload` and two backups, indexes 1 and 2, with the second time not earlier than the first. |
| `creating_leftover_is_restored_and_marked` | `db.sqlite.creating` and its `-wal`, with no live file, are moved into `broken/1-{millis}/`. The new file is empty and ready. The id is listed. |
| `creating_leftover_keeps_a_good_file` | A creating file beside a healthy `db.sqlite` is archived. Keys and visitors stay. `base.config` is not written. |
| `switch_of_a_broken_event_keeps_the_current_file` | Switching onto a broken file archives that file under its own `broken/1-{millis}/` and lists its id. The previous file still has its visitor and has no backup. |
| `switch_does_not_clear_the_reload_mark` | Switching to a healthy event leaves the earlier id in `full_reload`. |
| `failed_mark_does_not_drop_or_switch` | When `base.config` cannot be written, `switch` returns the previous event. The broken file is still the garbage bytes and no backup directory is created. |

### Done when

`cargo test --lib registration_server::event_db::tests::open` passes.

## Step 2. Load

[Back to summary](#summary)

Copy the Scala file into memory in the table `last_add` order. The first commit is the first five pages. Search and the list can run before the copy finishes.

### Work

1. Read the file in the same order as `table` `last_add`: `ORDER BY mem_u.ts DESC, mem_u.uid`. `uid` is ascending. Join `u_fulltext` for the list text, `emails` for the address, and `prints` for `MAX(ts)` and the count. A blank `gotsome` is `0`.
2. The first transaction inserts 100 `list_row` rows and their `u_fts` rows, the first five pages of 20, or fewer when the file has fewer visitors, then commits. Further transactions continue in that order. FTS5 automerge stays off until the last commit, which runs one merge.
3. After each commit, `loaded_count` is the committed count and `index_state` stays `loading` until the file is exhausted. A file of fewer than 100 rows commits once and is `ready`.
4. `open` leaves a non-empty file `loading` and does not copy inside `open`. `load_batch` copies the next batch and commits. The sync thread calls that function; tests call it on the calling thread, so the first commit is observable without a sleep.
5. A uid already in `list_row` is skipped. A write during the copy inserts it itself.

### Test scenarios

| Scenario | Assert |
|---|---|
| `first_page_before_the_rest` | After the first batch, `loaded_count` is 100, `index_state` is `loading`, and those rows are the first 100 of `table` `last_add` (733, then 3534 at row 20, 3535 at row 21, 795 last). uid 796 is absent. After the copy finishes, `loaded_count` is 5176 and `index_state` is `ready`. |
| `tied_timestamp_uses_table_uid_order` | Two visitors with the same `ts` are ordered by `uid` ascending, the same as `table`. The first of them is in the first batch. The second is not. |
| `existing_keys_stay` | `keys` is still `admin` and `IPTB2PA`. No third key is inserted. |
| `skip_uid_already_loaded` | uid 733 is inserted into memory before the batch that would have copied it. The finished load has one row for 733, and that row is the one inserted first. |
| `short_file_commits_once` | A file of 3 visitors commits in one batch and is `ready`. The newest uid is first. |

### Done when

`cargo test --lib registration_server::event_db::tests::load` passes, and step 1 stays green.

## Step 3. Search

[Back to summary](#summary)

Match the three text fields. Filter by category. Order by last add or last print.

### Work

1. `search(text, categories, order, limit, offset)` returns `total` and the page of `list_row`.
2. Empty text and empty categories returns every committed row in the requested order.
3. Text is the `MATCH` from the columns section. The `MATCH` string is a bound parameter. Empty text with categories reads `list_row.category` and does not query `u_fts`.
4. Text and categories apply both. The category test is on `list_row` after the match.
5. `limit` defaults to 20. `offset` defaults to 0.

### Test scenarios

| Scenario | Assert |
|---|---|
| `match_name` | Prefix `иль` returns uid 733. `Илья` returns uid 733 as well. |
| `match_surname_and_company` | `агранович` returns uid 733. `beautydrugs` returns uid 1. |
| `two_words_are_and` | `илья агранович` returns only uid 733. `илья` alone matches more than that one row. |
| `category_filter` | Category 7 returns 2,088 rows and does not return uid 249. The query plan reads `list_row`, not `u_fts`. |
| `category_and_text` | `агранович` plus category 7 returns uid 733. The same token plus category `-2` returns nothing. |
| `category_is_not_a_token` | Searching `7` does not return uid 733 and does not return 2,088 rows. |
| `order_last_add` | The first row is uid 733. |
| `order_last_print` | The first row is uid 249 with `last_print_ts` `1790784637258`. uid 733 has `last_print_ts` `0`. |
| `search_sees_committed_only` | After the first batch, `утимото` (uid 796, the first visitor after those 100) is absent and `эмирова` (uid 3535, inside those 100) is present. After `ready`, uid 796 is present. |
| `hostile_text_stays_a_query` | `"`, `'`, `*`, `:`, parentheses, `AND`, `OR`, `NOT`, `NEAR`, `^`, `\`, `%`, `_`, and a null byte return no error. `list_row` still has 5,176 rows and `агранович` still returns uid 733. |

### Done when

`cargo test --lib registration_server::event_db::tests::search` passes, and steps 1 and 2 stay green.

## Step 4. Write

[Back to summary](#summary)

A visitor write and a print both commit the file, then update memory.

### Work

1. `write` takes `uid`, the JSON string, and the list columns (`name`, `surname`, `c_name`, `category`, `ticket_status`, `gotsome`, `give_packet`, `added_ts`, `org_id`, `email`).
2. The file transaction upserts `mem_u` (`data`, `ts`, `category`, `ticket_status`, `org_id`) and the plain `u_fulltext` row, replaces that uid in `emails`, then commits.
3. The memory transaction upserts `list_row` and replaces that row in `u_fts`. `last_print_ts` and `print_count` stay as they were on an update. A new uid starts them at `0`.
4. `record_print(uid, ts)` inserts a `prints` row (`category` and `is_cert` may be null) and commits the file. Memory then sets `last_print_ts` to `ts` when `ts` is newer and increments `print_count`. There is no `prints_unique` table.
5. A write of a uid the loader has not copied yet is visible immediately, and the loader skips it.

### Test scenarios

| Scenario | Assert |
|---|---|
| `write_is_searchable` | On the copy, `write` a new uid. `search` by that surname returns it. The file `mem_u.data` is the JSON string. The 5,176 fixture rows are still there. |
| `write_replaces` | A second `write` of uid 733 changes the name. Search finds the new name and not `агранович`. One file row for 733. |
| `print_changes_order` | `record_print` on uid 733 with a `ts` newer than `1790784637258`. `last_print` lists 733 first and `print_count` is 1. The file has one new `prints` row. |
| `write_during_load` | After the first batch, `write` a new uid whose `added_ts` is older than uid 3535. When the copy is `ready`, that uid exists once and `loaded_count` is 5177. |

### Done when

`cargo test --lib registration_server::event_db::tests::write` passes, and steps 1–3 stay green.

## Step 5. Table

[Back to summary](#summary)

The list page is the columns the registration screen paints. `from` is the first row and `limit` is the page length, so page `n` is `from = n * limit`.

While the copy is still running, memory does not contain a full order. `table` reads the page and `total` from the file, so any page can be opened and the page count is the on-disk count. The rows of that page are inserted into `list_row` and `u_fts` when the uid is not already there. The sequential copy skips them, and search can match them. A uid already in memory is left as it is. When `index_state` is `ready`, `table` reads memory and does not query the file.

### Work

1. `table(categories, order, limit, from)` returns `total` and the page. `from` defaults to 0. `limit` defaults to 20.
2. Each row returns `uid`, `name`, `surname`, `c_name`, `category`, `ticket_status`, `gotsome`, `give_packet`, `added_ts`, `last_print_ts`, `print_count`, `org_id`, `email`.
3. When `index_state` is `ready`, the page is `search` with an empty string.
4. While `index_state` is `loading`, `total` is `COUNT(*)` of `mem_u` for that category filter, and the page is the same order run on the file: `LIMIT` and `from` as `OFFSET`.
5. Those file rows are copied into memory when missing. The copy does not replace a uid that is already in `list_row`.

### Test scenarios

| Scenario | Assert |
|---|---|
| `first_page` | After `ready`, `last_add`, limit 20. The first uid is 733, the page has 20 rows, the last of them is 3534, and `total` is 5176. |
| `second_page` | `from` 20 starts with uid 3535. |
| `table_category_and_last_print` | Category 7 and `last_print`. Every row has category 7. The first uid is 1253, who was printed, and uid 733 is not on that page. |
| `table_columns` | `from` 1027 is uid 732, with email `alfa23@inbox.ru`, `gotsome` `0`, `give_packet` `0`, and `print_count` `0` from `prints`. |
| `page_from_and_limit` | Before any batch, `from` 40 and `limit` 10 starts at uid 757 and has 10 rows. `from` 5160 has 16 rows. `from` 5176 is empty. `total` stays 5176, which is 259 pages of 20. |
| `loading_page_and_count_come_from_the_file` | Before any batch, `last_print` starts at uid 249. `total` is 5176. Those 20 rows are then in memory. |
| `file_row_wins_while_loading` | A memory row for 733 named `memory-only` does not change the file page. The page name is `илья`, and the memory name stays `memory-only`. |
| `asked_page_is_indexed` | `from` 100 starts at uid 796, search finds `утимото`, and after the copy finishes that uid exists once and `loaded_count` is 5176. |
| `ready_table_reads_memory` | After `ready`, changing the memory name of 733 changes the page. The file `u_fulltext` name stays `илья`. |

### Done when

`cargo test --lib registration_server::event_db::tests::table` passes, and steps 1–4 stay green.

## Step 6. Full row

[Back to summary](#summary)

The registration card reads `mem_u.data` from the file.

### Work

1. `full_row(uid)` returns the JSON string from the file.
2. An unknown uid returns none. It does not create a row.
3. `list_row` has no `data` column. The memory database is not queried for this call.

### Test scenarios

| Scenario | Assert |
|---|---|
| `full_row_is_the_json` | `full_row(733)` is the file JSON and contains `Илья`. The list row for 733 is still `илья`. |
| `full_row_missing` | An unknown uid returns none. |
| `list_row_has_no_json` | `list_row` columns do not include `data`. |

### Done when

`cargo test --lib registration_server::event_db` passes.
