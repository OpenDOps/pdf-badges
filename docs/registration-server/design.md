# registration-server

How this process shares one core. The remote login and the project token it reuses are [login-and-token.md](login-and-token.md). The order to port that login is [porting.md](porting.md). The credential file is [credentials.md](credentials.md). The event SQLite file is [db-design.md](db-design.md). How the remote probe becomes the corner signal is [network-quality.md](network-quality.md). How a badge is drawn and sent to CUPS is [printing.md](printing.md). The sync queue is the `event-sync` library in [event-sync/design.md](../event-sync/design.md). This process binds the full catalog.

The process that runs on the device at the venue. The machine is one weak core, on the order of a Wi-Fi router: small RAM, Linux, no spare CPU to share evenly. The process has three jobs.

1. Serve HTTP to people on the local network.
2. Send print jobs for those people.
3. Download registration data from a remote server, on a timer, from several endpoints.

Local HTTP and printing own the core. The remote download runs when that work is waiting on the network, and it gets out of the way when a local request or a print job becomes runnable.

Tokio can host this. Tokio treats every task on one runtime as equal, so the preference is in how the process is split, and in the OS scheduler, which already knows how to prefer one thread over another on a single core.

## Two threads

Two OS threads. Each thread runs its own current-thread Tokio runtime. There is no multi-thread runtime and no work-stealing between them.

```text
server thread          nice 0
  current-thread runtime
    HTTP + WebSocket
    print queue, one job at a time

sync thread            nice 15
  current-thread runtime
    timer
    up to SYNC_IN_FLIGHT downloads in flight (compiled, default 100)
    parse and store between socket reads
```

The server thread stays at nice 0. The sync thread calls `setpriority` on its own tid with nice 15 (Linux, including OpenWrt). When both threads are runnable, the kernel gives the core to the server thread. The sync thread runs in the gaps where the server thread is blocked in `epoll`: no local request to read, no print chunk to draw.

```rust
let server = std::thread::Builder::new().name("server".into()).spawn(|| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    rt.block_on(serve());
});

let sync = std::thread::Builder::new().name("sync".into()).spawn(|| {
    #[cfg(target_os = "linux")]
    unsafe {
        // Nice is per thread. This tid is the sync thread.
        let tid = libc::syscall(libc::SYS_gettid);
        libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 15);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(sync_loop());
});
```

`max_blocking_threads(1)` caps the blocking pool on the server runtime. The default pool is hundreds of threads, which is the wrong size for this machine. Sync work stays async and does not use that pool.

## What one core does with a download

A download spends almost all of its time waiting on a socket. While a task is parked on `.await`, it holds no CPU. The thread is in `epoll` and can run something else the moment a local socket becomes readable.

CPU is spent in the short stretches between those waits: the TLS handshake, copying bytes into a buffer, JSON parse, writing rows to local storage. On one core those stretches run one after another. Several endpoints in flight overlap their waits. They do not parse at the same time.

Firing every endpoint at once still creates one task per endpoint. The runtime polls a task until it awaits, then polls the next. When several responses arrive together, their read-and-parse stretches run in queue order. A local handler on the same runtime waits until the current stretch hits `.await`. Tokio's coop budget will force a yield after enough polls. That is fairness between equal tasks. It does not rank a registration lookup above a JSON parse.

A function that runs for a long time without `.await` — a full PDF render, a large `serde_json::from_slice` — occupies the thread until it returns. On the server thread, the accept loop does not run during that stretch. That is the case this split is built to avoid: heavy parse lives on the sync thread, and print work on the server thread yields between chunks.

## Remote sync

The queue is the `event-sync` library ([event-sync/design.md](../event-sync/design.md)). This process binds the full catalog, the event file, and the gate. Other hosts bind a shorter catalog and skip the gate.

The sync loop wakes on a timer and appends ready calls to one queue. The queue is as long as `SYNC_IN_FLIGHT`, which is also how many calls may wait on a response at once. It is compiled into the binary (unset is 100; `SYNC_IN_FLIGHT=2 cargo build` bakes another count). `--sync-in-flight` replaces that compiled value for one process. Downloads of the same path stay in order. Different paths, and uploads, share the queue. A free socket goes to a download that is waiting. An upload is sent on a socket that is still free. The core still parses one body at a time:

```rust
use futures::stream::{self, StreamExt};

// `ready` is the queue: at most `in_flight` calls. One path stays in order.
stream::iter(ready)
    .map(|call| async move { download(call).await })
    .buffer_unordered(in_flight)
    .for_each(|result| async move { store(result).await })
    .await;
```

The HTTP client pool is capped to the same width. Bodies are streamed into the store. A response is one read buffer plus the rows already written.

The sync loop also watches a gate shared with the server thread (`tokio::sync::watch`, or a semaphore of one permit). The gate closes while `inflight_requests > 0` or a print job is running. The sync task awaits the gate before it starts the next read-and-parse stretch. `nice` already hands the core to the server thread whenever that thread is runnable. The gate keeps a parse from starting in the quiet gap between two local requests.

A download that is preempted stays correct. The socket stays registered. The task is polled again when the server thread next blocks and the sync thread receives the core. The cycle finishes later. That is the intended trade.

## Local HTTP, WebSocket, and printing

The server runtime accepts HTTP and WebSocket on one socket. One current-thread runtime polls that accept loop, so a local call is server-thread work.

A request handler does its work and returns. It does not parse a remote payload and it does not render a whole document inline. Search reads the in-memory database from [db-design.md](db-design.md). The list page reads that database once the copy is ready. Until then a page turn reads the file, because the full order is not in memory yet, and copies that page into memory. The event file is the durable copy. A sync write is one transaction per row batch on that file, then the same batch is applied to the memory database.

An idle WebSocket is a socket wait. It does not hold the gate. The gate closes while a message from that socket is handled, and opens again when the handler returns to waiting for the next frame. An open socket would otherwise keep the sync thread paused for the whole connection.

One print job runs at a time. Further jobs wait on the print channel (`PRINT_QUEUE_DEPTH`, 16). The worker takes the next job only after the current job finishes. Between pages it awaits `tokio::task::yield_now()`, so the accept loop can take a request that arrived mid-document. A job that is already printing keeps the core until the next yield; the sync thread does not start a new parse while the gate is closed. What the worker renders and how it submits to CUPS is [printing.md](printing.md).

## What runs when

| Work | Server thread idle | A local request or a print chunk is runnable |
|---|---|---|
| Local HTTP and WebSocket | The server thread handles the socket as soon as it is readable | Same thread. The handler runs to its next `.await` with no sync task on this thread |
| Print | One job, yielding between pages | Owns the server thread until the next yield. The gate is closed |
| Remote download | Up to the compiled `SYNC_IN_FLIGHT` sockets in flight on the sync thread (default 100), shared with uploads. CPU only between reads | The sync thread is runnable and loses the core. In-flight reads sit in the kernel until the server thread blocks again |

## Rules that keep the core available

- The server thread and the sync thread are separate current-thread runtimes.
- HTTP and WebSocket share the server thread. An idle WebSocket does not hold the gate.
- The sync thread runs at `nice` 15, set on that thread's tid.
- The ready queue and the sockets share one cap: the compiled `SYNC_IN_FLIGHT` (default 100). `--sync-in-flight` replaces it for one process. The client pool matches that cap. Downloads of the same path stay in order. A download that is waiting to be sent takes a free socket ahead of an upload.
- Response bodies are streamed. The process does not hold every endpoint's body at once.
- The sync loop waits while a local request is in flight or a print job is running.
- The same thread probes the remote base every five seconds. That request does not close the gate. [network-quality.md](network-quality.md) is how two samples become the green, yellow, or red corner signal, and the timeout login and the event list use.
- One print job runs at a time, and it yields between pages.
- A long computation on the server thread awaits between chunks. `spawn_blocking` is reserved for a library call that has no yield point, and the blocking pool size is 1.

## Skeleton

The runnable skeleton is `src/registration_server`. `rust-reg registration-server` starts it.

```text
src/registration_server/
  mod.rs            two threads, config, serve()
  http.rs           HTTP routes and WebSocket
  grpc.rs           Registration service, compiled for tests only
  print_queue.rs    one job at a time, yield between pages
  sync.rs           event-sync queue: timer, compiled SYNC_IN_FLIGHT calls in flight (default 100), parse one body at a time
  net.rs            remote probe every five seconds, link quality
  gate.rs           closes while a local call or a print job is in progress
  store.rs          local registrations, locked for one batch
proto/irbis/registration/v1/registration.proto
```

HTTP and WebSocket listen on `--http` (default `0.0.0.0:8080`). `--endpoint` may be repeated. `--sync-every` is the period in seconds. The process does not open a gRPC port.

| Call | What the skeleton does |
|---|---|
| `GET /health` | `{"status":"ok"}` |
| `GET /registrations/{id}` | The stored row, or 404 |
| `POST /print` | `{"id","pages"}` onto the print queue |
| `GET /ws` | A text frame is a registration id. The reply is that row, or `{"found":false}` |

## Why this API stays HTTP

The clients on the venue network are browsers. A browser calls HTTP and decodes JSON with `JSON.parse`, which the engine already contains. Lookup and print are those HTTP routes. That is the fast client: no protobuf decoder in the page, no second protocol beside the socket that already serves the page.

The same two operations exist as gRPC `Lookup` and `EnqueuePrint` in `grpc.rs`. That file is compiled only when the crate is built for tests. `rust-reg registration-server` does not listen for them. Leaving the service in the device binary adds the generated stub, server reflection, and another accept loop, which is weight on a machine the size of a Wi-Fi router. Tests still start that listener and call the two methods, so the service stays covered without shipping it.

`rust-reg serve` is the PDF tooling process. It still speaks gRPC. That dependency stays in the crate for that process. It is not part of the venue registration server.
