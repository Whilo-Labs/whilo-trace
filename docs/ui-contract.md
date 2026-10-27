# Interface contract

How the interface (`app/ui`) talks to the core (`app/src`). The interface calls Tauri commands through
`base/api.js` and listens to one event. In a plain browser the same calls go to `base/mock.js`, which
answers with the mockup scenario, so the interface can be built and checked without the core.

Types live in `app/src/contract.rs`. JSON keys are camelCase. Command arguments are camelCase too
(Tauri maps `interval_s` to `intervalS`).

## Errors

Errors are short codes; the interface turns them into text (`HE.errorInfo`). A code with a detail after a
colon (`probe_unavailable: Access is denied.`) is read by the part before the colon; the detail is the
system's own message, in the system's language.

| Code | Comes from | Means |
|---|---|---|
| `invalid_host` | `add_target` | not a name or an IPv4 address as typed |
| `resolve_failed` | `add_target`, `Row.error` | the name was not found in DNS; a row keeps trying |
| `no_ipv4` | `add_target`, `Row.error` | the name (or a typed IPv6 address) has no IPv4 address; only IPv4 is probed |
| `probe_unavailable` | `add_target` (unknown probe type); `Row.error` as `probe_unavailable: <detail>` | probes of this type cannot be sent, the detail says why |
| `invalid_interval` | `add_target`, `set_interval` | not one of 0.5, 1, 2.5, 5, 10, 30, 60 |
| `no_such_target` | commands with a row id, `export_report` | the target or the row is gone |
| `no_such_hop` | `pin_hop`, `unpin_hop` | the TTL did not answer lately, or the pinned hop is gone |
| `not_implemented` | none now | kept for a command a build does not have |
| `db: <detail>` | any command, `Row.error` | the history database could not be read or written |
| `io: <detail>` | `export_report`, `open_folder`, `Row.error` | files and folders; anything else the core cannot sort |
| `core_unavailable` | the interface only | no snapshot came from the core; the core never sends it |

## Commands

| Command | Arguments | Returns |
|---|---|---|
| `snapshot` | `{ req: SnapshotReq }` | `Snapshot` |
| `add_target` | `{ req: { host, probe?, intervalS? } }` | `{ id }` (row id; an existing target returns its id). `intervalS`: one of 0.5, 1, 2.5, 5, 10, 30, 60, default 2.5 |
| `remove_target` | `{ id }` | `null`; history stays on disk. A pinned hop id unpins it |
| `set_paused` | `{ id, paused }` | `null` |
| `set_interval` | `{ id, intervalS }` | `null`; one of 0.5, 1, 2.5, 5, 10, 30, 60, else `invalid_interval` |
| `set_order` | `{ ids }` | `null`; row ids in the new order, kept between runs; unknown ids are skipped |
| `pin_hop` | `{ targetId, ttl }` | `{ id }`; `targetId` is the target's row id; the same hop pinned again keeps its id |
| `unpin_hop` | `{ id }` | `null` |
| `get_settings` | `{}` | `Settings` |
| `set_settings` | `{ settings: Partial<Settings> }` | `Settings`; values that do not fit are ignored |
| `recent_inputs` | `{}` | `string[]`, last hosts typed, newest first, at most 10 |
| `export_report` | `{ req: ReportReq }` | `{ dir, files }` |
| `pick_folder` | `{}` | `string` or `null` (folder dialog) |
| `open_folder` | `{ path }` | `null`; a folder opens in the file manager, a file of a report (`.html`, `.png`, `.csv`, `.json`) in its default program; any other path is `io: ...` |
| `set_background` | `{ dark }` | `null`; the window behind the page takes the page's background (`--bg`), so its edges stay in the theme while it grows. `base/theme.js` calls it when the shown theme changes; the core sets it from the saved theme at start |

Event `round`: a target finished a round, payload is its numeric target id. The interface asks for a new
snapshot at most once a second, and only while it shows live data (`base/live.js`).

A target saved on pause starts paused after a restart: no probe and no DNS lookup until `set_paused`
resumes it, then at its saved interval.

## SnapshotReq

```json
{ "window": "10m", "endMs": null, "range": null, "trace": null, "buckets": 120 }
```

- `window`: `5m`, `10m`, `30m`, `1h`, `6h`, `24h`.
- `endMs`: end of the window; `null` is live (ends now). Past mode sets it.
- `range`: `[fromMs, toMs)` picked on a timeline, half-open like the window: `toMs` is not in it, so a
  period of whole buckets ends where the next bucket starts. It covers the buckets from the one that holds
  `fromMs` to the one that holds `toMs - 1`, cut to the window. Every row and trace hop then also gets
  `rangeStats`; a period outside the window gets empty ones (`sent: 0`).
- `trace`: row id of the target on the trace screen; `null` on the main screen. A pinned hop id or an
  unknown id gives `trace: null`, not an error.
- `buckets`: N, default 120 (10 minutes in 5-second buckets, as in the mockups).

## Snapshot

```json
{
  "N": 120, "nowMs": 1793102400000, "fromMs": 1793101800000, "toMs": 1793102400000,
  "live": true, "bucketMs": 5000,
  "targets": [ Row, ... ],
  "trace": null,
  "history": { "fromMs": 1793016000000, "toMs": 1793102400000, "slots": 200, "loss": [8, 9, 31] },
  "loss": [],
  "settings": { "warnMs": 100, "badMs": 200, "theme": "system", "lang": "ru", "window": "10m" },
  "probesAvailable": ["icmp"], "ratePps": 18, "recent": ["1.1.1.1", "8.8.8.8"]
}
```

- Bucket `i` covers `[fromMs + i * bucketMs, fromMs + (i + 1) * bucketMs)`; the window is `[fromMs, toMs)`.
- `toMs` live: the end of the bucket that holds `nowMs`. That bucket is still filling, and `toMs` may be
  up to one bucket after `nowMs`. In the past: the end of the bucket that holds `endMs - 1`, so `toMs` is
  `endMs` or a little after it.
- `targets`: rows of the main screen in the user's order. A target has one row per probe type (only ICMP
  now). Pinned hops are rows too, with `hopOf` and `hop`.
- `trace`: only when `req.trace` is a target row id.
- `history`: 24 hours that end at `toMs`, 200 slots; a slot is listed when any row lost probes in it.
- `loss` and `ticks`: only the mock sends them (to reproduce the mockups); the interface computes time
  labels from `toMs` (`WT.ticksAt`).

### Row

```json
{
  "id": "t12", "targetId": 12, "host": "1.1.1.1", "name": "one.one.one.one", "ip": "1.1.1.1",
  "probe": "icmp", "interval": 2.5, "paused": false, "error": null,
  "series": [2.1, 2.3, null], "lo": [1.9, 2.0, null], "hi": [2.6, 2.8, null],
  "loss": [16, 17], "partial": [], "sentN": [2, 2, 2], "lostN": [0, 0, 2], "max": 10, "status": "loss",
  "stats": { "lossPct": 5.8, "noReplyPct": 0, "avg": 2.2, "min": 1.9, "max": 7.8, "last": 2.0,
             "lastLost": false, "jitter": 0.3, "sent": 240 },
  "rangeStats": { "lossPct": 0, "noReplyPct": 0, "avg": 2.1, "min": 1.9, "max": 2.6, "last": 2.0,
                  "lastLost": false, "jitter": 0.2, "sent": 24, "status": "ok" },
  "path": { "hops": 9, "origin": { "ttl": 7, "addr": "10.200.0.7" }, "silent": [2, 3, 4, 6],
            "partial": [{ "ttl": 1, "addr": "192.168.0.1", "name": null, "noReplyPct": 61 }] }
}
```

- `id`: stable row id. `t12` is target 12 with ICMP; `p7` is pinned hop 7. Pinned hop rows also have
  `"hopOf": "t12", "hop": 5` and no `path`.
- `host`: what the user typed; for a pinned hop its address.
- `name`: the second label. A target typed as an IP address: its reverse DNS name (`null` until it is
  known, or when there is none). A target typed as a name: the IPv4 address its latest round went to
  (`null` before the first round). A pinned hop: its reverse DNS name.
- `ip`: the address the rounds go to (a pinned hop: the hop's address).
- `paused`: pinned hop rows of a paused target are paused too.
- `error`: what keeps the row from getting data, a code from the table above (`resolve_failed`, `no_ipv4`,
  `probe_unavailable: <detail>`, `db: <detail>`, `io: <detail>`); `null` once rounds come again.
  `status` stays what the data says (often `ok` with no data), so the interface shows the error instead.
  Pinned hop rows have `error: null`; their target's row carries it.
- `series`, `lo`, `hi`: average, min and max RTT in ms per bucket; `null` where nothing answered.
- `loss`: buckets drawn red. `partial`: buckets drawn amber (hops only). See the rules below.
- `sentN`, `lostN`: per bucket, the probes the row got (rounds that probed it; 0 for a bucket without
  rounds) and how many of them got no answer (loss on the path and a hop's own misses alike). They give
  the exact "k of n" in hints.
- `max`: top of the latency scale, ms.
- `status`: `ok`, `loss`, `partial`, `silent`, `noreply` (names and hints: `WT.statuses`). A target row
  is `loss` from 2 % lost rounds to the destination, `noreply` when none of its rounds reached it, else
  `ok`; a pinned hop has the status of its hop in the trace.
- `stats`: the whole window. `rangeStats`: the picked period, when `req.range` is set.
- `stats.sent`, `rangeStats.sent`: rounds of the period that probed the row: for a target every round of
  the period, for a hop the rounds with a probe for its TTL (a round stops at the destination, so a hop
  past a shorter route has fewer). `lossPct` and `noReplyPct` are shares of `sent`.
- `avg`, `min`, `max`, `last`, `jitter`: `null` when nothing answered in the period.
- `rangeStats.status`: the status over the picked period, by the same rules as `status` applied to the
  rounds of the period only. Only in `rangeStats`: without a period the status is `status`, and the
  interface does not compute its own.
- `path`: short summary of the route for target rows: route length, the hop where loss starts, hops silent
  for this probe type, hops that answer only some probes (whole window).

### Trace

```json
{ "target": "t12", "rounds": 240, "hops": [ {
  "ttl": 7, "addr": "10.200.0.7", "name": null, "status": "loss", "origin": true, "dest": false,
  "max": 10, "series": [...], "lo": [...], "hi": [...], "loss": [16, 17], "partial": [],
  "sentN": [...], "lostN": [...], "byProbe": { "icmp": "loss" }, "stats": { ... },
  "rangeStats": { ..., "status": "loss" } } ] }
```

- `rounds`: rounds of the window. The rounds of a picked period are the target row's `rangeStats.sent`.
- Hops have the same fields as rows; `status`, `origin` and `dest` are for the whole window, the status
  over a picked period is `rangeStats.status`.

## Bucket rules

- N buckets of `bucketMs` cover `[fromMs, toMs)`. Rounds fall into the bucket of their start time.
- Target row: a bucket is in `loss` when at least one round in it did not reach the destination.
- Hop (pinned row or trace hop): a bucket is in `loss` when the hop did not answer in a round where the
  destination was lost too (loss on the path); it is in `partial` when the hop did not answer while the
  destination answered. Red is only for loss on the path.
- The latency line goes through replies only (`null` buckets break it).
- Silent hop: a hop that never answers in the window while a later hop answers is `silent`. Its `series`,
  `lo`, `hi` are all `null`, it has no `loss` or `partial` marks, its `lossPct` and `noReplyPct` are 0
  (its misses say nothing about the path) and `avg`, `min`, `max`, `last`, `jitter` are `null`; the
  interface hatches it. Over a picked period the same goes for a hop with `rangeStats.status: "silent"`.
  The target row lists such hops in `path.silent`. A hop that never answers while nothing after it
  answers either is `noreply` and keeps its loss.
- Paused rows keep their history; buckets after the pause are `null` and not loss.

## Settings

`{ warnMs: 100, badMs: 200, theme: "system" | "light" | "dark", lang: "ru" | "en", window: "10m" }`.
Thresholds paint the latency background; language and theme apply to the whole window. `window` is the
window both screens open with: the main screen and the trace screen save it with `set_settings`.

## ReportReq

```json
{ "rows": ["t12", "p7"], "fromMs": 1793101800000, "toMs": 1793102400000,
  "formats": ["html", "png", "csv", "json"], "tzOffsetMin": 240,
  "findings": ["1.1.1.1, ICMP: потери до цели 5,8 % за 240 раундов, начинаются на хопе 7 (10.200.0.7)."],
  "pngBase64": "data:image/png;base64,...", "dir": null }
```

Returns `{ "dir": "C:\\Users\\...\\Documents\\Whilo Trace\\whilo-trace-20261027-160000", "files": ["report.html", ...] }`.

- The period is `[fromMs, toMs)`, like the window.
- `findings` are written into the report exactly as the dialog shows them, in the interface language;
  `[]` (or no key) writes no findings. The engine's own findings, in English, are only for the command
  line (`whilo-trace-cli report`).
- `dir`: the folder the user picked, `null` for `Documents/Whilo Trace`. The report always goes into a
  new folder inside it, `<name>-<YYYYMMDD-HHMMSS>`, and the answer's `dir` is that new folder. `<name>`
  is the target's host when every row belongs to one target, else `whilo-trace`; the time is `toMs` in
  `tzOffsetMin`. A folder that exists already is not overwritten: the next one is `<name>-<time>-2`.
- `files`: what was written, of `report.html`, `timeline.png` (only with `pngBase64`), `hops.csv` and
  `samples.csv` (`csv`), `report.json`.
- The HTML page is in the interface language. Status names are neutral and name the probe type of the
  data, as the interface does (`не отвечает на ICMP`, `No reply to ICMP`); CSV and JSON are in English
  with times in UTC.
