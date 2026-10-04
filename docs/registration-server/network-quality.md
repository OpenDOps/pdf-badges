# Network quality

The sync thread probes the remote base every five seconds. That request does not close the local-request gate. [design.md](design.md) is why the probe sits on that thread.

`GET /api/network` reports the reading. The login page and the event page show it as a signal in the top-right corner. The corner stays empty until two probes have come back.

## What one probe is

The probe is an HTTP GET of the remote base. Any response headers count as connected. A timeout, a refused connection, or a dropped socket is a failed sample. The probe gives up after four seconds. It is not ICMP.

Samples sit in a window of eight. The signal uses only the last two.

## The signal

Three colors. The sentence next to the dot is the Russian catalog (`network.good`, `network.fair`, `network.poor`, `network.offline`, `network.local`).

| Last two probes | JSON | Corner |
|---|---|---|
| Both ok, slower one at most 300 ms | `quality: "good"` | Green. Good connection. |
| Both ok, slower one at most 1.5 s | `quality: "fair"` | Yellow. Not so good. A warning. |
| Both ok, slower than 1.5 s | `quality: "poor"` | Red. Bad connection. |
| One failed | `quality: "poor"` | Red. Bad connection. |
| Both failed | `offline: true`, `quality: null` | Red. No connection with the internet server. |
| The page cannot read `GET /api/network` | — | Red. No connection with the local server. The sentence says it is probably turned off. |
| Fewer than two | `quality: null`, `offline: false` | Hidden. |

`fair` is the only yellow warning. `poor` and offline are the same red signal. Offline is the case where both of the last two probes failed.

## Timeouts

`Link::budget` is the one reading later remote calls use. `timeout_secs` is the whole seconds in the JSON. `limit` is that same budget on the HTTP call. Login, the event list, and choosing an event all take it from there. Those three calls also share one in-flight guard: a second call while the first is still running returns an in-progress error and does not open another socket.

| Last two probes | Quality | Timeout |
|---|---|---|
| Not enough samples yet | hidden | 10 s |
| Both failed | offline | 2 s |
| One failed | poor | 10 s |
| Both ok, slower one at most 300 ms | good | 5 s |
| Both ok, slower one at most 1.5 s | fair | 8 s |
| Both ok, slower than 1.5 s | poor | 20 s |

`POST /api/login` and `POST /api/events/select` return that `timeout_secs` on success and on failure. The login button, and the event choice, stay disabled and count down one second at a time for that period. A bind that does not store a token returns its error code, and the event screen shows that sentence.

## Logging

`--debug-ping` prints each probe to stderr, then the quality those samples produced. A line looks like `registration-server PING https://kuprin.su/ 200 1600ms quality=poor offline=false samples=2 timeout=20s`. `--debug` does not include those lines. `--debug` prints HTTP requests, login, the event list, select, sync, and print. Passwords, cookies, and the project token are not in those lines. Without `--debug-ping` the probe stays quiet.
