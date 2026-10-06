# Event database

How the Scala client opens SQLite, what the second connection actually is, and what the Rust process should open instead. [porting.md](porting.md) step 4 is this initialization. The build sequence is [db-implementation-plan.md](db-implementation-plan.md). The helper that attaches the project token to later remote calls is the step after it. [design.md](design.md) is how the server thread and the sync thread share the core once the file and the memory database are open.

The Scala sources are `RegBoxHelpers.scala` (`init`, `setCurrentExpoUID`, `initExpoData`, `useSqlite`) and `PersistenceHelper.scala` (`prepareDatabase`).

## When the Scala client opens the file

`./base/main.props` holds `current_expo_uid`. The database path is `./base/{uid}/db.sqlite`.

Process start loads the properties file. If `current_expo_uid` is non-empty, `init` calls `initExpoData` and, when `keys` is empty, inserts one admin key and one operator key. The token is not required for that open. The sync timer starts only when both the uid and `curentr_expo_token` are present.

`setCurrentExpoUID` writes the uid and the token, zeroes the sync cursors, calls `initExpoData`, then inserts the same two keys when `keys` is empty.

`initExpoData` runs when the uid changed. It closes both connections, then calls `prepareDatabase` when the uid is non-empty. `cleanCurrentExpoUIDToken` deletes the uid, the token, and the name, and leaves both connections open on the previous event.

## Two connections

`useSqlite` holds one pair for the life of the event and runs every call under one lock on `RegBoxHelpers`.

| connection | URL | lifetime |
|---|---|---|
| event file | `jdbc:sqlite:./base/{uid}/db.sqlite` | closed when the event changes |
| working connection | `jdbc:sqlite:` | same |

The working URL has an empty file name. SQLite treats that as a private temporary database and deletes it when its last connection closes. The field that holds it is `inMemoryOpenedConnection`. The comment above that field describes memory being reclaimed on close. The database is a temporary file, kept alive because that connection stays open. It is not `jdbc:sqlite::memory:`.

A third file, `./base/org{orgId}/dborg.sqlite`, holds `managers` and org `users`. `prepareDatabase` opens it, copies rows into the working connection, and closes it. Later manager writes open it again.

`useSqlite` passes the working connection and a `Set` of both connections. Call sites that mean "the event file" take `both.head`.

## What `prepareDatabase` copies

The event file is created with `CREATE TABLE IF NOT EXISTS`. The working connection gets its own tables, then rows are inserted with `String.format` batches.

| data | event file | working connection |
|---|---|---|
| `mem_u` | full row, including JSON `data`. `errors` is `TEXT`. `new_id` is added later as `INTEGER` | same columns except `data`. `errors` is `INTEGER` (empty or not). `new_id` is `TEXT` |
| `u_fulltext` | ordinary table | FTS3 virtual table |
| `prints` | one row per print | `prints_unique`, a count per uid |
| `barcodes`, `keys`, `lottery_wins` | source | full copy |
| `man_to_user` | `uid INTEGER` | `uid TEXT` |
| `phones`, `emails` | tables | Scala `HashMap`s, not SQL |
| `managers`, org `users` | org file | `managers` table and FTS3 `org_u_fulltext` |
| `scans`, `ticketbarcodes`, `log`, `print_stat`, `scanner_ids`, `expo_days` | event file only | absent |

Lookups (visitor search, barcode take, key check) read the working connection. The JSON body is read from the connection taken as the event file. Writes that must survive restart are issued on both connections, as two statements. `createKey` inserts on both and returns one of the two generated ids.

## What holds up

The event file is the copy that survives restart. `prepareDatabase` rebuilds the working connection from it, so a restart drops anything that existed only on the working connection.

Splitting "rows we search" from "JSON we store" matches the machine: search and barcode checks should not drag the visitor payload through every lookup. FTS is why the working side exists as SQL at all. The durable `u_fulltext` is a plain table; the working side is the index.

Keeping one connection open for the event, and closing it when the event changes, is the right lifetime. Default keys on an empty `keys` table are part of that init, in both `init` and `setCurrentExpoUID`.

## What does not hold

The working connection is a temporary file that the rest of the code treats as a second source of truth.

- `both.head` is `Set.head`. Which connection that is depends on hash order, and the disk-only reads (`SELECT data FROM mem_u`) and several disk-only writes use it.
- The two connections commit separately. A crash between them leaves them different until the next `prepareDatabase`, which recopies from the event file and drops a write that reached only the working connection.
- The schemas differ (`errors`, `new_id`, `man_to_user.uid`, FTS3 versus a plain table). A statement written for one side fails or means something else on the other.
- The load builds SQL text. `useSqlite` turns a thrown exception into `None`.
- The copy of every visitor, barcode, and fulltext row runs under the same lock as a lookup, in the process that also serves HTTP.
- Phones and emails are a third copy, outside both databases.
- Logout does not close the file.

A cache rebuilt from the event file on every open is what search and the registration list need. The OpenWrt flash is too slow for a full-text query or a page turn to read it. Issuing each mutation twice, without one transaction, makes that cache a second source of truth. The cache stays. The file commits first, and the cache is updated from that commit.

## What the Rust process opens

Two databases for one event. `rusqlite` opens both.

| database | where | what it answers |
|---|---|---|
| `base/{event_id}/db.sqlite` | the flash file | durable rows, including `mem_u.data` |
| `file:{name}?mode=memory&cache=shared` | RAM | full-text search and the registration list |

The file URL in the Scala client is `jdbc:sqlite:` with an empty name. That is a temporary file. On a router that file can land on the same flash the cache was meant to avoid. `mode=memory` is RAM only. `cache=shared` lets the server thread and the sync thread open the same name and see the same pages. The name is unique to that open, so the next event does not share those pages. The memory database lives while at least one of those connections is open. Closing them on an event change drops it.

The credential file stores `event_id` only together with `project_token`. Startup opens both databases when that binding is present and starts the memory load in the background. Choosing an event writes the binding, opens the new file, and then closes the previous pair. If the new file cannot be opened, the previous event stays open. Clearing the binding closes both and stops the load. The Scala client could open on a uid with no token; this process does not, because that document is already refused.

A new file is written as `db.sqlite.creating` and renamed to `db.sqlite` after the schema commits. A live file that does not start with a SQLite header, or is shorter than the header, is moved without being opened, so its `-wal` and `-shm` stay as they were. A longer file is checked with `PRAGMA quick_check`. Its sidecar files are copied before that open, because SQLite truncates them when the file is not a database, and the backup keeps the copy. The moved files land in `{event_id}/broken/{index}-{unix_millis}/`. The index starts at 1 and increases for each later drop. The time is unix milliseconds at the drop. The original names are kept in that directory so the backup can be opened for repair. An empty file is then created at `db.sqlite`. A leftover `db.sqlite.creating` with no live file is archived the same way. A healthy live file is kept. A leftover creating file next to it is archived and does not count as a drop.

Each replaced database is recorded in `{base}/base.config`:

```yaml
version: 1
full_reload:
  - GELCPAQSFL
```

`full_reload` lists the event ids whose file was replaced. `needs_full_reload` is true when that list is not empty. `open` does not clear an id. A later sync clears it after that event has been downloaded again. A first create and a healthy reopen do not write the file. If the config cannot be written, the broken file is left in place and is not archived, so the mark and the backup stay together.

On the file, WAL is on, `synchronous` is `FULL`, and `busy_timeout` covers one write batch. Each thread keeps its own file connection. A sync write is one transaction per row batch. After that transaction commits, the same batch is applied to the memory database. A crash before the memory update loses nothing that was committed: the next open loads memory from the file again.

The file tables are the event-file tables from `prepareDatabase`, including `mem_u.new_id INTEGER` once that column has been added. A Scala file such as `GELCPAQSFL` opens without a rewrite.

- `mem_u` keeps `data`. The Scala indexes are `uid` and `org_id`. `open` adds indexes on `code`, `barcode`, `repl_barcode`, and `in_synch` when they are missing. That is the only change applied to an existing file.
- `u_fulltext` is an ordinary table: `uid`, `name`, `surname`, `c_name`, `category`, `ticket_status`, `gotsome`, `give_packet`. The file never creates an FTS virtual table, and no query runs `MATCH` against it. The table is the reload source for the memory index, so a restart copies these columns and does not parse every visitor JSON. The Scala file is already this shape. FTS3 exists only on its working connection. This event stores the list names in lowercase; the original case stays inside `mem_u.data`.
- `prints` stays the log: `uid`, `ts`, `category`, `is_cert`. `prints_unique` is not a file table. Load computes `print_count` and `last_print_ts` from `prints`.
- `keys`, `barcodes`, `ticketbarcodes`, `scans`, `phones`, `emails`, `scanner_ids`, `print_stat`, `log`, `expo_days`, `lottery_wins`, `man_to_user` are the same tables. `man_to_user.uid` stays `INTEGER`, as on the Scala file. `phones` and `emails` stay tables. An existing `keys` table is left as it is. An empty one gets one admin row and one operator row.

The memory database holds two tables. Search text is the only thing in the full-text index. Category filtering and both list orders are ordinary indexes, because an FTS5 table has no btree: a `WHERE category = ?` on it would read every row.

`list_row` is a normal table:

| column | role |
|---|---|
| `name`, `surname`, `c_name` | the tokens `getUsersQuery` matches (`name:`, `surname:`, `c_name:`). Stored here in display case |
| `category` | integer. A filter is equality, OR of several values. Btree index. Not a token |
| `added_ts` | `mem_u.ts`. Order `last_add`, descending. Btree index |
| `last_print_ts` | `MAX(prints.ts)`, or `0` when the visitor has never been printed. Order `last_print`, descending, so a never-printed row sorts last. Btree index |
| `print_count` | how many times the badge was printed. The Scala column `prints_unique.ts` stores this count, not a time |
| `uid`, `ticket_status`, `gotsome`, `give_packet`, `org_id`, `email` | the list page. No search index |

`gotsome` is the local `0`/`1` mark `POST /setgotsome` writes. Sync does not fill it. When the event uses ticket payment, a badge prints if the ticket is paid or this mark is set. `give_packet` is `1` when the visitor JSON has an `ext_packets` entry with `has > 0`.

`u_fts` is FTS5 with `content='list_row'` and columns `name`, `surname`, `c_name` only. The tokenizer is `unicode61`, so match is case-folded and the stored text stays in display case. A search joins `u_fts` to `list_row` for the category test and the order. A category filter with an empty search reads `list_row` and does not touch `u_fts`. Each search word is a quoted phrase with a prefix `*` outside the quotes, and that `MATCH` text is a bound parameter. A word with no letter or digit is not sent to FTS.

`mem_u.data` stays on the file. Opening one visitor reads that one JSON row from flash.

The Scala unfiltered list is `db_view_buf`. It is filled in `mem_u.ts` order. A save and a print both move that uid to the front, and a restart throws the buffer away and sorts by `mem_u.ts` again. `GET /api/registrations` keeps the prints at the front: newest `last_print_ts` first, and a visitor who was never printed follows in `added_ts` order. A search and a category use that same order. `last_add` remains the added-time sort.

Barcode pools were also read from the working connection. When that slice arrives, the memory database holds them too, so taking the next barcode does not read flash.

## Loading the memory database

`open` returns when the file is open and the memory schema exists. The copy runs on the sync thread. List and search do not wait for it and do not take a loading flag. They read whatever is already committed. A screen that wants a progress mark calls `index_state`, which is `loading` or `ready`.

The copy reads the file in the table `last_add` order: `ORDER BY mem_u.ts DESC, mem_u.uid`, with `uid` ascending. The first transaction is 100 rows, the first five pages of 20, then it commits. A file with fewer visitors commits those rows and is `ready`. Further rows commit in that same order. Each commit is visible. Search reads those committed rows and does not query the file.

The list page does not use that partial order. `last_add` and `last_print` are a total order only when every visitor is in memory. Until `index_state` is `ready`, `table` reads the page and the count from the file, so any `from`/`limit` page can be opened and the page count is the on-disk count. The rows of the page that was asked for are inserted into `list_row` and `u_fts` when that uid is not already there. The background copy skips them, and search can match them. A uid already in memory is not replaced from the file. Neighboring pages are not prefetched. When the copy is `ready`, `table` reads memory and a page turn does not read the flash.

A memory database cannot use WAL, so a reader waits out the batch that is open. The batches stay short for that reason. FTS5 automerge stays off during the copy, and one merge runs when the copy finishes. A `MATCH` in the middle is an ordinary query on the committed rows. It does not read the flash file and it does not build a second index. The cost is that wait, plus a few more index segments until the final merge.

A visitor saved while the copy is still running is written to the file and inserted into memory immediately. The copy skips a uid that is already there, so the new row is not dropped and is not inserted twice.

The org file stays a later file, opened when an org id is configured. Its search index is the same kind of memory FTS (`org_u_fulltext`). Its rows are not copied into the event file.

When `keys` is empty after the schema is applied, insert one admin row and one operator row. The original key is seven characters from `0123456789ABCDEFGHIKLMNOPQRSTVXYZ` (`generateKey`). The HTTP routes that list and edit keys stay the admin plan that follows step 3. This step only makes the rows exist.

This step does not download visitors, barcodes, or config. Later slices write those tables through the token helper.
