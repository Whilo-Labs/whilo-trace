# Interface contract

How the interface (`app/ui`) talks to the core (`app/src`). The interface calls Tauri commands through
`base/api.js` and listens to two events. In a plain browser the same calls go to `base/mock.js`, which
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
| `io: <detail>` | `export_report`, `open_folder`, `save_image`, `geo_remove`, `Row.error`, `Snapshot.geo.download.error` | files and folders; anything else the core cannot sort |
| `geo_busy` | `geo_download`, `geo_remove` | a download of the places database is running already |
| `geo_unavailable` | `geo_download`, `geo_remove` | no updates here: the simulated network (`WHILO_TRACE_SIMULATE`) never reaches DB-IP, and its places are made up |
| `geo_invalid: <detail>` | `geo_download`, `Snapshot.geo.download.error` | not a month, not newer than the database in use, or a downloaded file that is not the database it should be |
| `geo_cancelled` | `Snapshot.geo.download.error`, event `geo` | the user stopped the download (`geo_cancel`) |
| `net: <detail>` | `Snapshot.geo.download.error`, event `geo` | no connection to DB-IP: no network, a timeout, TLS |
| `http: <status>` | `Snapshot.geo.download.error`, event `geo` | DB-IP answered with this status (`http: 404`: the month is not out) |
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
| `geo_check` | `{}` | `{ offer, reason }`: whether a newer places database is out (see Places database). At most one check a day goes to the server, answered or not: after a check that failed (no network, a timeout, a server error) the later calls that day answer `cached`. None on the simulated network |
| `geo_download` | `{ month }` | `null` at once; the download runs on its own and reports through the event `geo` and `Snapshot.geo.download`. Only a month newer than the database in use; `geo_busy`, `geo_unavailable`, `geo_invalid: ...` |
| `geo_cancel` | `{}` | `null`; the running download ends as `cancelled` |
| `geo_remove` | `{}` | `null`; deletes the downloaded database and goes back to the bundled one. The days of the last check and offer stay: an update offered or checked today is not offered again today; `geo_busy` during a download |
| `save_image` | `{ dir, name, pngBase64 }` | `{ path }`; the picture of the map as `<name>.png` in `dir` (`null`: Documents/Whilo Trace, as for reports). Never overwrites a file (see below); `io: ...` |

Event `round`: a target finished a round, payload is its numeric target id. The interface asks for a new
snapshot at most once a second, and only while it shows live data (`base/live.js`). On the trace screen a
round of the shown target also starts the route map's packet: the first snapshot asked after the event
(`Live.askedAt`) tells how that round ended.

Event `geo`: a download of the places database goes on; the payload is the `download` object of
`Snapshot.geo` (below). It comes when the download starts, at most every 250 ms while it runs, and once
when it ends (`done`, `failed` or `cancelled`), also in past mode, when no snapshots come.

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
  "settings": { "warnMs": 100, "badMs": 200, "theme": "system", "lang": "ru", "window": "10m",
                "geoCheck": true, "home": null },
  "probesAvailable": ["icmp"], "ratePps": 18, "recent": ["1.1.1.1", "8.8.8.8"],
  "geo": { "state": "ok", "month": "2026-10", "source": "bundled",
           "download": { "state": "running", "month": "2026-11", "received": 1234, "total": 65000000,
                         "error": null } }
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
- `geo`: the places database of the route map (DB-IP City Lite and ASN Lite), in every snapshot.
  - `state`: `ok`; `missing` (there is none: the program came without one, or it was deleted);
    `unreadable` (the files are there but do not open; found out when they are first opened, for a trace
    or by `geo_check` or `geo_download`, `ok` until then); `simulated` (the simulated network: every
    place is made up, nothing is downloaded).
  - `month`: release month of the database in use, `null` without one. With `unreadable`, the month of
    the set that does not open.
  - `source`: `bundled` (came with the program), `downloaded` (an update, kept in `geo-update` in the
    data folder), `simulated`; `null` without a database.
  - `download`: the last download of this run, `null` before one and after `geo_remove`. `state`:
    `running`, `done`, `failed`, `cancelled`. `received` and `total` are bytes of both files; `total`
    is `null` when the server does not tell their sizes. `error`: an error code (`net: ...`,
    `http: 404`, `geo_invalid: ...`, `io: ...`, `geo_cancelled`) or `null`. When a download is `done`,
    the snapshots show the new `month` and `source` at once.

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
            "partial": [{ "ttl": 1, "addr": "192.168.0.1", "name": null, "noReplyPct": 61 }],
            "unreachable": [] }
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
- `status`: `ok`, `loss`, `partial`, `silent`, `noreply`, `unreachable` (names and hints: `WT.statuses`).
  A target row is `loss` from 2 % lost rounds to the destination, `noreply` when none of its rounds
  reached it, else `ok`; a pinned hop has the status of its hop in the trace. `unreachable` is a hop that
  answered "destination unreachable" to at least 2 % of its probes: the path ends at that router, and it
  is never the destination.
- `stats`: the whole window. `rangeStats`: the picked period, when `req.range` is set.
- `stats.sent`, `rangeStats.sent`: rounds of the period that probed the row: for a target every round of
  the period, for a hop the rounds with a probe for its TTL (a round stops at the destination, so a hop
  past a shorter route has fewer). A round that did not reach the destination is kept up to where it
  ended (one silent hop after the last answer, or a router that answered "destination unreachable"), yet
  its probes to the hops from there to the destination were sent and got nothing back: it counts as sent
  and lost on the path at each of them. The destination for this is where the rounds of the period that
  reached it ended, else those of the whole window; without such rounds a hop counts only the rounds that
  probed it. `lossPct` and `noReplyPct` are shares of `sent`.
- `avg`, `min`, `max`, `last`, `jitter`: `null` when nothing answered in the period.
- `rangeStats.status`: the status over the picked period, by the same rules as `status` applied to the
  rounds of the period only. Only in `rangeStats`: without a period the status is `status`, and the
  interface does not compute its own.
- `path`: short summary of the route for target rows: route length, the hop where loss starts, hops silent
  for this probe type, hops that answer only some probes, routers that say the destination is unreachable
  (`unreachable`, as `origin`: ttl and address; whole window). `rangePath`: the same over the picked
  period, on target rows with `rangeStats`.

### Trace

```json
{ "target": "t12", "rounds": 240, "hops": [ {
  "ttl": 7, "addr": "10.200.0.7", "name": null, "status": "loss", "origin": true, "dest": false,
  "max": 10, "series": [...], "lo": [...], "hi": [...], "loss": [16, 17], "partial": [],
  "sentN": [...], "lostN": [...], "byProbe": { "icmp": "loss" }, "stats": { ... },
  "rangeStats": { ..., "status": "loss" },
  "geo": { "level": "own", "special": "private", "city": null, "region": null, "cc": null,
           "country": null, "lat": null, "lon": null, "asn": null, "asOrg": null, "distKm": null,
           "fiberMs": null, "vacuumMs": null, "rttMs": null, "extraMs": null, "verdict": null } },
  { "ttl": 9, "addr": "192.0.2.77", ...,
    "geo": { "level": "city", "special": null, "city": "Frankfurt am Main", "region": "Hesse",
             "cc": "DE", "country": "Германия", "lat": 50.11, "lon": 8.68, "asn": 64504,
             "asOrg": "Transit DE", "distKm": 3350.2, "fiberMs": 33.53, "vacuumMs": 22.35,
             "rttMs": 52.4, "extraMs": 18.87, "verdict": "ok" } } ],
  "home": { "lat": 40.41, "lon": 49.87, "city": "Baku", "cc": "AZ", "country": "Азербайджан",
            "source": "auto", "ttl": 3 } }
```

- `rounds`: rounds of the window. The rounds of a picked period are the target row's `rangeStats.sent`.
- Hops have the same fields as rows; `status`, `origin` and `dest` are for the whole window, the status
  over a picked period is `rangeStats.status`.
- `home`: where the path starts on the route map, `null` without one. `source: "auto"`: the first hop in
  TTL order placed in a city; never the destination, nor a hop that answered with the target's address.
  `ttl` is that hop. `source: "manual"`: `settings.home`; `city`, `cc`, `country` and `ttl` are `null`
  then. `lat` and `lon` are rounded to 0.01°. Distances on the map count from here.
- `hops[].geo`: the place of the hop and its speed-of-light check. `null` for a hop without an address,
  and on every hop when the trace has no target address (no destination hop with an address and no `ip`
  on the row); the interface places such hops, and hops with `level: "none"`, between their neighbours.
  - `level`: `own` (the home network or the provider's carrier-grade NAT: drawn at home, never looked
    up), `city`, `country` (only the country is known), `none` (no place). Without a usable database
    (`Snapshot.geo.state` `missing` or `unreadable`) every hop with an address still has `geo`: `own`
    hops as always, the others `none`.
  - `special`: set only for hops that were not looked up: `own` hops (`private`, `cgnat`, `loopback`,
    `link-local`) and, outside the simulated network, `none` hops in other special-purpose ranges
    (`documentation`, `benchmarking`, `multicast`, `reserved`, `unspecified`, `broadcast`). A hop that
    was looked up has `special: null`, also a simulated router in a documentation range: classify hops by
    `level`, not by `special`.
  - `city`, `region`: as the database has them (English in DB-IP Lite). `cc`: ISO 3166 alpha-2.
    `country`: in the interface language (`settings.lang`; Russian falls back to English).
  - `lat`, `lon`: the point the database gives, rounded to 0.01°, kept when the check finds it
    impossible. A `country` hop may have a point or not.
  - `asn`, `asOrg`: the autonomous system, from the ASN database.
  - `distKm`, `fiberMs`, `vacuumMs`, `rttMs`, `verdict`: only for `city` hops with a home and a round
    trip: the great-circle distance from home (0.1 km), the shortest round trip over it in fibre (light
    at c/1.5) and in vacuum, the round trip checked and the verdict (ms to 0.01). `rttMs` is the hop's
    minimum over the window (`stats.min`), never over a picked period. `verdict`: `ok`; `doubtful`
    (faster than light in fibre goes there and back, with 1 ms of tolerance); `impossible` (faster than
    light in vacuum). The home hop is 0 km away and `ok`. `country`, `own` and `none` hops get no check.
  - `extraMs`: how much longer than light in fibre the answer took; only with `verdict: "ok"`.

## Bucket rules

- N buckets of `bucketMs` cover `[fromMs, toMs)`. Rounds fall into the bucket of their start time.
- Target row: a bucket is in `loss` when at least one round in it did not reach the destination.
- Hop (pinned row or trace hop): a bucket is in `loss` when the hop did not answer in a round where the
  destination was lost too (loss on the path); it is in `partial` when the hop did not answer while the
  destination answered. Red is only for loss on the path. A round that ended before the hop on its way to
  the destination (see `stats.sent`) is loss on the path there, and so is "destination unreachable" from
  the router before the destination at the destination's TTL.
- A round that stopped at the known length and whose deepest probe expired on the way (the route got
  longer) is not reported at all: it could not have reached the destination and says nothing about loss.
- The latency line goes through replies only (`null` buckets break it).
- Silent hop: a hop that never answers in the window while a later hop answers is `silent`. Its `series`,
  `lo`, `hi` are all `null`, it has no `loss` or `partial` marks, its `lossPct` and `noReplyPct` are 0
  (its misses say nothing about the path) and `avg`, `min`, `max`, `last`, `jitter` are `null`; the
  interface hatches it. Over a picked period the same goes for a hop with `rangeStats.status: "silent"`.
  The target row lists such hops in `path.silent`. A hop that never answers while nothing after it
  answers either is `noreply` and keeps its loss.
- Paused rows keep their history; buckets after the pause are `null` and not loss.

## Settings

`{ warnMs: 100, badMs: 200, theme: "system" | "light" | "dark", lang: "ru" | "en", window: "10m",
geoCheck: true, home: null | { lat, lon } }`.
Thresholds paint the latency background; language and theme apply to the whole window. `window` is the
window both screens open with: the main screen and the trace screen save it with `set_settings`.
`geoCheck`: look for a newer places database when the map opens (`geo_check`). `home`: a home put on the
map by hand, in degrees rounded to 0.1°; `null` uses the automatic one (`Trace.home`). In `set_settings`
a missing `home` key leaves it as it is, `"home": null` clears it, and a point off the globe
(|lat| > 90 or |lon| > 180) is ignored like any value that does not fit.

## ReportReq

```json
{ "rows": ["t12", "p7"], "fromMs": 1793101800000, "toMs": 1793102400000,
  "formats": ["html", "png", "csv", "json"], "tzOffsetMin": 240,
  "findings": ["1.1.1.1, ICMP: потери до цели 5,8 % за 240 раундов, начинаются на хопе 7 (10.200.0.7)."],
  "pngBase64": "data:image/png;base64,...", "dir": null,
  "mapPngBase64": "data:image/png;base64,..." }
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
  `samples.csv` (`csv`), `report.json`, and last `map.png` (only with `mapPngBase64`).
- `mapPngBase64`: the route map drawn by the interface, a PNG as base64 or a `data:` URL; optional. The
  folder gets it as `map.png`, and `report.html` shows the same picture inline (the page stays one file)
  in a «Карта маршрута» / "Route map" figure above the charts, with the credit
  `IP Geolocation by DB-IP (https://db-ip.com), CC BY 4.0` in its footer. Anything that is not a PNG is
  left out, as a `pngBase64` that does not decode is. Only the trace screen sends it, when its route has
  places, with any formats: drawn from the data on that screen when the report is saved, light, nothing
  hidden.
- The HTML page is in the interface language. Status names are neutral and name the probe type of the
  data, as the interface does (`не отвечает на ICMP`, `No reply to ICMP`); CSV and JSON are in English
  with times in UTC.

## Places database

`geo_check` answers `{ "offer": null | { "month": "2026-11", "bytes": 61234567 }, "reason": "..." }`.
The core goes through these in order and stops at the first that holds:

| `reason` | When | Request |
|---|---|---|
| `simulated` | the simulated network | never |
| `disabled` | `settings.geoCheck` is off | none |
| `asked` | an update was offered today already | none |
| `cached` | the server was asked today already, answered or not | none |
| `current` | the database in use is of the current month (UTC), or later | none |
| `offer` | the server has the current month: `offer` is set | one (two without HEAD) |
| `none` | the server has nothing newer yet | one (two without HEAD) |
| `missing` | there is no database (or it does not open), and the server has none out for this month yet | one (two without HEAD) |
| `error` | the check failed (no network, a timeout, a server error); the interface shows nothing. The day still counts: later calls that day answer `cached` | one (two without HEAD) |

- One check is a HEAD of the City file of the current month on download.db-ip.com; a server that
  answers it with 405 or 501 gets a second request, a GET of its first byte (`Range: bytes=0-0`). With
  no database at all the check still goes out, and an offer downloads the current month.
- `bytes`: the compressed size of the City file when the server tells it, else `null`; the ASN file adds
  about 5 MB. A size hint only.
- Days are local dates, kept in `geo-update/state.json` in the data folder: the day of the last check
  sent to the server, answered or not (`cached`), and of the last offer (`asked`). The day of a check is
  written before its request goes out, so a check that failed, or one cut short by closing the program,
  uses up the day too; when the file cannot be written, the next call asks again. A call that sends no
  request writes nothing. `geo_remove` keeps them.
- `geo_download` fetches the City and ASN files of `month` into `geo-update`, unpacked and checked,
  under new names; the new database is used as soon as the download is `done`, and older downloaded
  months are deleted. The bundled database (`geo` among the program's files) is never written to.

`save_image` writes `<name>.png`: in `name`, every character but ASCII letters, digits, `-`, `_` and `.`
becomes `_`, dots and underscores at either end are dropped, a `.png` at the end is not doubled, at most
120 characters are kept, and Windows device names (`CON`, `NUL`, ...) get a `_` in front; an empty name
is `whilo-trace-map`. A file of
that name is never overwritten: the picture becomes `<name>-2.png`, `<name>-3.png` and so on. `pngBase64`
is a PNG as base64 or a `data:` URL; anything else is `io: not a PNG picture`. `open_folder` opens the
saved picture or its folder.
