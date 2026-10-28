// Browser stand-in for the Rust core: answers the same commands as the app (docs/ui-contract.md)
// with the mockup scenario, so every part can be built and checked in a plain browser.
// Not used inside the app (window.__TAURI__ is there). Address flags, comma separated: ?mock=empty (first run),
// ?mock=live (a round event every 2.5 s, data shifts as if time goes on), ?mock=strict (no trace on the main
// screen, exactly like the app), ?mock=down (the core does not answer: every command fails with
// core_unavailable until MOCK.down = false is set in the console).
// Places database of the route map: ?mock=geo-new (a newer month is out, the check offers it), geo-none (no
// database: no places, the map shows its empty state), geo-broken (the database does not read).
// Settings live in sessionStorage, so every tab has its own.
(function () {
  if (window.__TAURI__) return;
  var WT = window.WT;
  var Q = new URLSearchParams(location.search);
  var FLAGS = (Q.get('mock') || '').split(',');
  WT.N = 120; // the mockup window: 10 minutes in 5-second buckets

  // Scenario: a home line with loss of about 5% that starts after the provider's CGNAT hop.
  // Every address except the public resolvers (1.1.1.1, 8.8.8.8) and the home router is made up: documentation,
  // shared and private ranges (RFC 5737, RFC 6598, RFC 1918).
  // Loss lands on the same moments for every target behind the provider hop: a path problem.
  var LOSS = [16, 17, 43, 70, 71, 97, 109];
  var G_LOSS = [16, 43, 44, 70, 97, 109, 110]; // the google row and its demo route (TRACE_G), from hop 7 on
  var END = new Date(2026, 9, 27, 16, 0, 0).getTime(), END0 = END;
  var TICKS = {
    '5m': ['15:55', '15:56', '15:57', '15:58', '15:59', '16:00'],
    '10m': ['15:50', '15:52', '15:54', '15:56', '15:58', '16:00'],
    '30m': ['15:30', '15:36', '15:42', '15:48', '15:54', '16:00'],
    '1h': ['15:00', '15:12', '15:24', '15:36', '15:48', '16:00'],
    '6h': ['10:00', '11:12', '12:24', '13:36', '14:48', '16:00'],
    '24h': ['вчера 16:00', 'вчера 20:48', '01:36', '06:24', '11:12', '16:00'],
  };
  var HISTORY = [8, 9, 31, 55, 56, 57, 88, 120, 121, 140, 141, 142, 143, 150, 163, 170, 171, 186, 190, 196];

  function S(spec) { return WT.series(spec); }
  // Rows of the main screen in the user's order. hopOf marks a hop pinned from a trace.
  // The TCP 443 row exercises the multi-probe layout; the real engine has ICMP only for now.
  var rows = FLAGS.indexOf('empty') >= 0 ? [] : [
    { id: 'one', targetId: 1, host: '1.1.1.1', name: 'one.one.one.one', ip: '1.1.1.1', probe: 'icmp', interval: 2.5, max: 10,
      series: S({ seed: 11, base: 2.1, noise: 0.5, spikes: [[58, 7.8], [88, 5.6]] }), loss: LOSS, status: 'loss' },
    { id: 'one-tcp', targetId: 1, host: '1.1.1.1', name: 'one.one.one.one', ip: '1.1.1.1', probe: 'tcp443', interval: 2.5, max: 10,
      series: S({ seed: 12, base: 2.4, noise: 0.6, spikes: [[58, 8.2]] }), loss: [16, 43, 70, 97, 109], status: 'loss' },
    { id: 'google', targetId: 2, host: '8.8.8.8', name: 'dns.google', ip: '8.8.8.8', probe: 'icmp', interval: 2.5, max: 80,
      series: band(23, 53.1, 54.6, 67.4), loss: G_LOSS, status: 'loss' },
    { id: 'steam', targetId: 3, host: 'store.steampowered.com', name: '198.51.100.26', ip: '198.51.100.26', probe: 'icmp', interval: 2.5, max: 150,
      series: S({ seed: 37, base: 36, noise: 3, spikes: [[61, 128], [62, 104]] }), loss: [17, 43, 70, 71, 97, 110], status: 'loss' },
    { id: 'upwork', targetId: 4, host: 'www.upwork.com', name: '198.51.100.104', ip: '198.51.100.104', probe: 'icmp', interval: 2.5, max: 250,
      series: S({ seed: 41, base: 58, noise: 4, spikes: [[84, 231], [85, 188]] }), loss: [16, 43, 70, 97, 109], status: 'loss' },
    { id: 'router', targetId: 1, host: '192.168.0.1', name: 'роутер', ip: '192.168.0.1', hopOf: 'one', hop: 1, probe: 'icmp', interval: 2.5, max: 2,
      series: S({ seed: 5, base: 0.6, noise: 0.3 }), partial: WT.sample(77, 0.64), status: 'partial' },
    { id: 'cgnat', targetId: 1, host: '100.64.10.1', name: 'isp-cgnat-1', ip: '100.64.10.1', hopOf: 'one', hop: 5, probe: 'icmp', interval: 2.5, max: 5,
      series: S({ seed: 9, base: 1.2, noise: 0.4, spikes: [[52, 3.1]] }), loss: [], status: 'ok' },
  ];
  rows.forEach(function (r) { r.paused = false; r.error = null; });

  // Trace of 1.1.1.1 over the window (rounds are counted per snapshot from the window and the interval).
  // byProbe holds the status per probe type.
  var TRACE = {
    target: 'one', rounds: 240,
    hops: [
      { ttl: 1, addr: '192.168.0.1', name: 'роутер', status: 'partial', max: 2, series: S({ seed: 5, base: 0.6, noise: 0.3 }), partial: WT.sample(77, 0.64), byProbe: { icmp: 'partial', tcp443: 'partial' } },
      { ttl: 2, addr: null, status: 'silent', byProbe: { icmp: 'silent', tcp443: 'silent' } },
      { ttl: 3, addr: null, status: 'silent', byProbe: { icmp: 'silent', tcp443: 'ok' } },
      { ttl: 4, addr: null, status: 'silent', byProbe: { icmp: 'silent', tcp443: 'silent' } },
      { ttl: 5, addr: '100.64.10.1', name: 'isp-cgnat-1', status: 'ok', max: 5, series: S({ seed: 9, base: 1.2, noise: 0.4, spikes: [[52, 3.1]] }), loss: [], byProbe: { icmp: 'ok', tcp443: 'ok' } },
      { ttl: 6, addr: null, status: 'silent', byProbe: { icmp: 'silent', tcp443: 'silent' } },
      { ttl: 7, addr: '10.200.0.7', status: 'loss', origin: true, max: 10, series: S({ seed: 13, base: 2.0, noise: 0.5, spikes: [[58, 6.9]] }), loss: LOSS, byProbe: { icmp: 'loss', tcp443: 'loss' } },
      { ttl: 8, addr: '203.0.113.131', status: 'loss', max: 10, series: S({ seed: 17, base: 2.2, noise: 0.5, spikes: [[58, 7.4]] }), loss: LOSS, byProbe: { icmp: 'loss', tcp443: 'loss' } },
      { ttl: 9, addr: '1.1.1.1', name: 'one.one.one.one', status: 'loss', dest: true, max: 10, series: S({ seed: 11, base: 2.1, noise: 0.5, spikes: [[58, 7.8], [88, 5.6]] }), loss: LOSS, byProbe: { icmp: 'loss', tcp443: 'loss' } },
    ],
  };
  TRACE.hops.forEach(function (h) { if (!h.series) { h.series = new Array(WT.N).fill(null); h.loss = []; h.partial = []; } });

  // A made-up series around a minimum and an average (the minimum once at bucket 60, the maximum at bucket 30),
  // for the demo route: its numbers are the ones of the approved route map mockup.
  function band(seed, min, avg, max) {
    var r = WT.rng(seed), v = [];
    for (var i = 0; i < WT.N; i++) v.push(Math.round((min + r() * 2 * (avg - min)) * 100) / 100);
    v[60] = min;
    v[30] = max;
    return v;
  }
  // Demo route of the route map for the google row (Дизайн/Whilo Trace карта маршрута/common/demo-route.js): Baku to
  // 8.8.8.8 through Tbilisi, Istanbul and Frankfurt, documentation addresses as on the simulated network. Loss starts
  // at hop 7; the database puts hop 8 where light cannot get in its time, knows only the country of hop 9, and puts
  // 8.8.8.8 (anycast) in Mountain View, as real databases do.
  var TRACE_G = {
    target: 'google', rounds: 240,
    hops: [
      { ttl: 1, addr: '192.168.1.1', status: 'ok', max: 5, series: band(101, 0.6, 0.9, 4.1), loss: [] },
      { ttl: 2, addr: '100.64.0.1', status: 'ok', max: 10, series: band(102, 1.8, 2.3, 6.0), loss: [] },
      { ttl: 3, addr: '198.51.100.17', name: 'bras1.bak.example.net', status: 'ok', max: 10, series: band(103, 2.4, 2.9, 7.7), loss: [] },
      { ttl: 4, addr: '198.51.100.94', name: 'core2.bak.example.net', status: 'partial', max: 10, series: band(104, 2.8, 3.4, 9.0), loss: [],
        partial: WT.sample(105, 0.22).filter(function (i) { return i !== 30 && i !== 60; }) },
      { ttl: 5, addr: null, status: 'silent' },
      { ttl: 6, addr: '203.0.113.5', name: 'ge-0-1.tbs1.example.org', status: 'ok', max: 20, series: band(106, 9.4, 10.2, 15.8), loss: [] },
      { ttl: 7, addr: '203.0.113.66', name: 'et-3.ist1.example.org', status: 'loss', origin: true, max: 50, series: band(107, 31.0, 32.4, 41.2), loss: G_LOSS },
      { ttl: 8, addr: '203.0.113.130', status: 'loss', max: 50, series: band(108, 36.2, 37.5, 46.9), loss: G_LOSS },
      { ttl: 9, addr: '192.0.2.10', status: 'loss', max: 60, series: band(109, 39.0, 40.6, 51.3), loss: G_LOSS },
      { ttl: 10, addr: '192.0.2.77', name: 'ae1.fra2.example.com', status: 'loss', max: 80, series: band(110, 52.4, 53.9, 66.0), loss: G_LOSS },
      { ttl: 11, addr: '8.8.8.8', name: 'dns.google', status: 'loss', dest: true, max: 80, series: band(23, 53.1, 54.6, 67.4), loss: G_LOSS },
    ],
  };
  TRACE_G.hops.forEach(function (h) {
    h.byProbe = { icmp: h.status };
    if (!h.series) { h.series = new Array(WT.N).fill(null); h.loss = []; h.partial = []; }
  });
  function traceOf(host) { return host === '8.8.8.8' ? TRACE_G : TRACE; }

  // Short path summary that every row carries (the core sends it with each target): route length, where the
  // loss starts, hops silent for this probe type, hops that answer only some probes, routers that say the
  // destination is unreachable.
  function pathOf(probe, tr) {
    var o = tr.hops.filter(function (h) { return h.origin; })[0];
    return {
      hops: tr.hops.length,
      origin: o ? { ttl: o.ttl, addr: o.addr } : null,
      silent: tr.hops.filter(function (h) { return (h.byProbe[probe] || h.status) === 'silent'; }).map(function (h) { return h.ttl; }),
      partial: tr.hops.filter(function (h) { return (h.byProbe[probe] || h.status) === 'partial'; }).map(function (h) {
        return { ttl: h.ttl, addr: h.addr, name: h.name || null, noReplyPct: (h.partial || []).length / WT.N * 100 };
      }),
      unreachable: tr.hops.filter(function (h) { return (h.byProbe[probe] || h.status) === 'unreachable'; }).map(function (h) {
        return { ttl: h.ttl, addr: h.addr };
      }),
    };
  }
  rows.forEach(function (r) { if (!r.hopOf) r.path = pathOf(r.probe, traceOf(r.host)); });

  var settings = { warnMs: 100, badMs: 200, theme: 'system', lang: 'ru', window: '10m', geoCheck: true, home: null };
  try { Object.assign(settings, JSON.parse(sessionStorage.getItem('wt-mock-settings') || '{}')); } catch (e) { /* no storage */ }
  var recent = ['1.1.1.1', '8.8.8.8', 'store.steampowered.com', 'www.upwork.com'];
  var seq = 100, listeners = {};

  function copy(x) { return JSON.parse(JSON.stringify(x)); }
  function pad2(n) { return (n < 10 ? '0' : '') + n; }
  // 20261027-160000 in the zone of the report, as report::file_stamp in the core.
  function fileStamp(ms, offsetMin) {
    var d = new Date(ms + offsetMin * 60000);
    return d.getUTCFullYear() + pad2(d.getUTCMonth() + 1) + pad2(d.getUTCDate()) + '-' + pad2(d.getUTCHours()) + pad2(d.getUTCMinutes()) + pad2(d.getUTCSeconds());
  }
  function setOf(list) { var s = {}; (list || []).forEach(function (i) { s[i] = true; }); return s; }
  function sum(list, i0, i1) { var s = 0; for (var i = i0; i <= i1; i++) s += list[i] || 0; return s; }
  // Rounds of a line in every bucket (sentN): a round starts every interval seconds and counts in the bucket of
  // its start time, as in the core. When rounds are rarer than buckets, some buckets have none.
  function roundsIn(interval, from, bucket) {
    var iv = (interval > 0 ? interval : 2.5) * 1000, out = [];
    for (var i = 0; i < WT.N; i++) out.push(Math.ceil((from + (i + 1) * bucket) / iv - 1e-9) - Math.ceil((from + i * bucket) / iv - 1e-9));
    return out;
  }
  // Probes without a reply in every bucket (lostN): a loss bucket lost them all (its series is null), a partial
  // bucket about half (the hop answered the rest), a bucket without a reply (a silent hop) all of them.
  function lostIn(t, sentN) {
    var loss = setOf(t.loss), part = setOf(t.partial), v = t.series || [];
    return sentN.map(function (n, i) { return loss[i] || v[i] == null ? n : part[i] ? Math.ceil(n / 2) : 0; });
  }
  // Same formula as WT.stats, restricted to buckets i0..i1: what the core returns as stats (the whole window)
  // and rangeStats (a picked period). sent: rounds in those buckets.
  function statsIn(t, i0, i1) {
    var v = t.series, n = i1 - i0 + 1, lossSet = setOf(t.loss), partSet = setOf(t.partial);
    var seen = [], lost = 0, part = 0, last = null, lastLost = false;
    for (var i = i0; i <= i1; i++) {
      if (lossSet[i]) lost++;
      if (partSet[i]) part++;
      if (!lossSet[i] && !partSet[i] && v[i] != null) { seen.push(v[i]); last = v[i]; lastLost = false; } else lastLost = true;
    }
    var total = seen.reduce(function (a, b) { return a + b; }, 0), jit = 0;
    for (var k = 1; k < seen.length; k++) jit += Math.abs(seen[k] - seen[k - 1]);
    return { lossPct: lost / n * 100, noReplyPct: part / n * 100, avg: seen.length ? total / seen.length : null,
      min: seen.length ? Math.min.apply(null, seen) : null, max: seen.length ? Math.max.apply(null, seen) : null,
      last: last, lastLost: lastLost, jitter: seen.length ? jit / Math.max(1, seen.length - 1) : null,
      sent: t.sentN ? sum(t.sentN, i0, i1) : n };
  }
  // Status for a picked period by the core's rule for the window: loss from 2 % (LOSS_NOTABLE_PCT), else ok.
  // Other statuses stay (silent, without reply, says the destination is unreachable); a hop that answers only
  // some probes stays partial while the period has such buckets.
  function rangeStatus(t, s) {
    if (t.status !== 'loss' && t.status !== 'ok' && t.status !== 'partial') return t.status;
    if (s.lossPct >= 2) return 'loss';
    return t.status === 'partial' && s.noReplyPct > 0 ? 'partial' : 'ok';
  }
  // A line as the core sends it: per-bucket probe counts, no latency in loss buckets (partial ones keep it),
  // min and max per bucket, stats for the window and, with a picked period, rangeStats.
  function finish(t, interval, from, bucket, idx) {
    var loss = setOf(t.loss), v = (t.series || []).map(function (x, i) { return loss[i] ? null : x; });
    t.series = v;
    t.lo = v.map(function (x) { return x == null ? null : Math.round(x * 90) / 100; });
    t.hi = v.map(function (x) { return x == null ? null : Math.round(x * 125) / 100; });
    t.sentN = roundsIn(interval, from, bucket);
    t.lostN = lostIn(t, t.sentN);
    t.stats = statsIn(t, 0, WT.N - 1);
    if (idx) { t.rangeStats = statsIn(t, idx[0], idx[1]); t.rangeStats.status = rangeStatus(t, t.rangeStats); }
  }
  function toIdx(ms, from, bucket) { return Math.max(0, Math.min(WT.N - 1, Math.floor((ms - from) / bucket))); }

  // ---------- places database of the route map (Snapshot.geo, Trace.home, hops[].geo; docs/ui-contract.md) ----------
  // The mock database knows the addresses of the two traces, as the simulated network's made-up places do: city and
  // region in English (as DB-IP Lite has them), the country in the interface language. Its documentation addresses
  // stand in for public routers (the core looks such addresses up only on the simulated network); every other address
  // is classified as the core does.
  // address -> [city, region, cc, lat, lon, asn, asOrg]; a country-only record has no city.
  var PLACE = {
    '198.51.100.17': ['Baku', 'Baku', 'AZ', 40.41, 49.87, 64500, 'Demo provider'],
    '198.51.100.94': ['Baku', 'Baku', 'AZ', 40.41, 49.87, 64500, 'Demo provider'],
    '203.0.113.5': ['Tbilisi', 'Tbilisi', 'GE', 41.72, 44.83, 64501, 'Transit GE'],
    '203.0.113.66': ['Istanbul', 'Istanbul', 'TR', 41.01, 28.98, 64502, 'Transit TR'],
    '203.0.113.130': ['Ashburn', 'Virginia', 'US', 39.04, -77.49, 64502, 'Transit TR'],
    '192.0.2.10': [null, null, 'BG', 42.73, 25.49, 64503, 'Transit BG'],
    '192.0.2.77': ['Frankfurt am Main', 'Hesse', 'DE', 50.11, 8.68, 64504, 'Transit DE'],
    '8.8.8.8': ['Mountain View', 'California', 'US', 37.39, -122.08, 15169, 'Google LLC'],
    '1.1.1.1': ['San Francisco', 'California', 'US', 37.77, -122.42, 13335, 'Cloudflare, Inc.'],
  };
  var COUNTRY = { AZ: ['Azerbaijan', 'Азербайджан'], GE: ['Georgia', 'Грузия'], TR: ['Turkey', 'Турция'], US: ['United States', 'США'],
    BG: ['Bulgaria', 'Болгария'], DE: ['Germany', 'Германия'] };
  var GEO_NOW = '2026-10'; // the month of the mock's clock (END)
  var GEO0 = {
    month: FLAGS.indexOf('geo-none') >= 0 ? null : FLAGS.indexOf('geo-new') >= 0 ? '2026-09' : GEO_NOW,
    broken: FLAGS.indexOf('geo-broken') >= 0,
  };
  var GEO = { month: GEO0.month, source: GEO0.month ? 'bundled' : null, broken: GEO0.broken, download: null, asked: false, timer: null };
  var GEO_CITY = 131457024, GEO_ASN = 5000000; // compressed sizes of the two files the mock's server has
  // As the core: a set that does not open is unreadable and keeps its month and source; no set at all is missing.
  function geoState() {
    var d = GEO.download ? copy(GEO.download) : null;
    if (GEO.broken) return { state: 'unreadable', month: GEO.month, source: GEO.source, download: d };
    if (!GEO.month) return { state: 'missing', month: null, source: null, download: d };
    return { state: 'ok', month: GEO.month, source: GEO.source, download: d };
  }
  function geoUsable() { return !!GEO.month && !GEO.broken; }
  function geoEmit() { var d = copy(GEO.download); (listeners.geo || []).forEach(function (cb) { cb(d); }); }
  // A command called without an argument it needs: what Tauri answers before the core runs.
  function badArgs(cmd, key) { return 'invalid args `' + key + '` for command `' + cmd + '`: command ' + cmd + ' missing required key ' + key; }
  // The special-purpose range of an IPv4 address as the core classifies it (crates/geo special.rs), null for a public one.
  function special(a) {
    var p = a.split('.').map(Number), a0 = p[0], a1 = p[1], a2 = p[2];
    if (a === '0.0.0.0') return 'unspecified';
    if (a === '255.255.255.255') return 'broadcast';
    if (a0 === 0) return 'reserved';
    if (a0 === 10 || (a0 === 172 && a1 >= 16 && a1 <= 31) || (a0 === 192 && a1 === 168)) return 'private';
    if (a0 === 100 && a1 >= 64 && a1 <= 127) return 'cgnat';
    if (a0 === 127) return 'loopback';
    if (a0 === 169 && a1 === 254) return 'link-local';
    if ((a0 === 192 && a1 === 0 && a2 === 2) || (a0 === 198 && a1 === 51 && a2 === 100) || (a0 === 203 && a1 === 0 && a2 === 113)) return 'documentation';
    if (a0 === 198 && (a1 === 18 || a1 === 19)) return 'benchmarking';
    if ((a0 === 192 && a1 === 0 && a2 === 0) || (a0 === 192 && a1 === 88 && a2 === 99)) return 'reserved';
    if (a0 >= 224 && a0 <= 239) return 'multicast';
    if (a0 >= 240) return 'reserved';
    return null;
  }
  var OWN = { private: 1, cgnat: 1, loopback: 1, 'link-local': 1 };
  function geoRec(level, sp, p, en) {
    var c = p && COUNTRY[p[2]];
    return {
      level: level, special: sp || null, city: p ? p[0] : null, region: p ? p[1] : null, cc: p ? p[2] : null,
      country: c ? c[en ? 0 : 1] : null, lat: p ? p[3] : null, lon: p ? p[4] : null, asn: p ? p[5] : null, asOrg: p ? p[6] : null,
      distKm: null, fiberMs: null, vacuumMs: null, rttMs: null, extraMs: null, verdict: null,
    };
  }
  // The place of one address before the check (crates/geo enrich.rs): own classes are drawn at home and never looked
  // up; other special ranges are not looked up either (special set, level none), except the mock's stand-ins for
  // public routers. Without a usable database a looked-up address has no place (level none). Only IPv4 here.
  function locate(addr, en) {
    if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(addr)) return null;
    var sp = special(addr), p = geoUsable() ? PLACE[addr] : null;
    if (sp && OWN[sp]) return geoRec('own', sp, null, en);
    if (sp && !PLACE[addr]) return geoRec('none', sp, null, en);
    return p ? geoRec(p[0] ? 'city' : 'country', null, p, en) : geoRec('none', null, null, en);
  }
  function r2(v, k) { return Math.round(v * k) / k; }
  // To 0.1 degree as the core rounds a home (half away from zero), never -0.
  function tenth(v) { var k = Math.round(Math.abs(v) * 10) / 10; return (v < 0 ? -k : k) + 0; }
  function onGlobe(p) { return !!p && typeof p.lat === 'number' && typeof p.lon === 'number' && isFinite(p.lat) && isFinite(p.lon) && Math.abs(p.lat) <= 90 && Math.abs(p.lon) <= 180; }
  // Great-circle distance (R = 6371 km) and the shortest round trip in vacuum (crates/geo physics.rs); fibre is 1.5
  // times slower, and the check has 1 ms of tolerance.
  function distKm(a, b) {
    var rad = Math.PI / 180, pa = a.lat * rad, pb = b.lat * rad, dp = pb - pa, dl = (b.lon - a.lon) * rad;
    var h = Math.pow(Math.sin(dp / 2), 2) + Math.cos(pa) * Math.cos(pb) * Math.pow(Math.sin(dl / 2), 2);
    return 2 * 6371 * Math.asin(Math.min(1, Math.sqrt(h)));
  }
  function vacuumMs(km) { return 2000 * km / 299792.458; }
  // Places, home and the check for a finished trace, as the core does after the per-target loop. The target is the
  // destination hop's address, else the row's (rowIp); without one no hop gets a place. Home: the one set by hand, else
  // the first hop placed in a city that is neither the destination nor answers with the target's address. The check
  // uses each hop's minimum over the window; the home hop is 0 km away and ok. A hop without a place has geo: null.
  function geoFill(tr, rowIp) {
    tr.home = null;
    tr.hops.forEach(function (h) { h.geo = null; });
    var dh = tr.hops.filter(function (h) { return h.dest && h.addr; })[0], target = dh ? dh.addr : rowIp;
    if (!target) return;
    var en = settings.lang === 'en', mh = settings.home, home = null, from = null;
    tr.hops.forEach(function (h) { if (h.addr) h.geo = locate(h.addr, en); });
    if (onGlobe(mh)) {
      home = { lat: r2(mh.lat, 100), lon: r2(mh.lon, 100), city: null, cc: null, country: null, source: 'manual', ttl: null };
    } else {
      from = tr.hops.filter(function (h) { return !h.dest && h.addr !== target && h.geo && h.geo.level === 'city'; })[0] || null;
      if (from) home = { lat: from.geo.lat, lon: from.geo.lon, city: from.geo.city, cc: from.geo.cc, country: from.geo.country, source: 'auto', ttl: from.ttl };
    }
    tr.home = home;
    if (!home) return;
    tr.hops.forEach(function (h) {
      var g = h.geo, rtt = h.stats ? h.stats.min : null;
      if (!g || g.level !== 'city') return;
      if (!(rtt != null && isFinite(rtt) && rtt >= 0)) rtt = null;
      if (h !== from && rtt == null) return;
      var d = h === from ? 0 : distKm(home, g), vac = vacuumMs(d), fib = vac * 1.5;
      g.distKm = r2(d, 10); g.fiberMs = r2(fib, 100); g.vacuumMs = r2(vac, 100);
      if (rtt != null) {
        g.verdict = rtt + 1 < vac ? 'impossible' : rtt + 1 < fib ? 'doubtful' : 'ok';
        g.rttMs = r2(rtt, 100); g.extraMs = g.verdict === 'ok' ? r2(Math.max(0, rtt - fib), 100) : null;
      }
      if (h === from) g.verdict = 'ok';
    });
  }
  // The trace the core sends for a row: 1.1.1.1 rows get TRACE, the google row the demo route.
  function traceFor(id) {
    var r = rows.filter(function (x) { return x.id === id; })[0];
    if (r && r.host === '1.1.1.1') return copy(TRACE);
    if (r && r.host === '8.8.8.8') return copy(TRACE_G);
    return null;
  }

  function snapshot(req) {
    req = req || {};
    var win = req.window || settings.window || '10m', span = WT.windowSeconds[win] * 1000;
    var to = req.endMs || END, from = to - span, bucket = span / WT.N;
    var out = {
      N: WT.N, nowMs: END, fromMs: from, toMs: to, live: !req.endMs, bucketMs: bucket,
      targets: copy(rows),
      // The core sends the trace only for req.trace. The mockups also read it on the main screen,
      // so the mock keeps it there unless ?mock=strict (strict is how the app behaves).
      trace: req.trace ? traceFor(req.trace) : (FLAGS.indexOf('strict') >= 0 ? null : copy(TRACE)),
      history: { fromMs: to - 86400000, toMs: to, slots: 200, loss: HISTORY.slice() },
      loss: LOSS.slice(), ticks: TICKS, settings: copy(settings), probesAvailable: ['icmp'],
      ratePps: rows.filter(function (r) { return !r.paused; }).length ? 18 : 0, recent: recent.slice(),
      geo: geoState(),
    };
    // The mockup labels are for 16:00; once time moves (?mock=live) or the past is shown, WT.ticksAt(toMs) makes them.
    if (to !== END0) delete out.ticks;
    // range is [fromMs, toMs) like the window: toMs is the end of the last picked bucket
    var idx = req.range ? [toIdx(req.range[0], from, bucket), toIdx(req.range[1] - 1, from, bucket)] : null;
    out.targets.forEach(function (t) { finish(t, t.interval, from, bucket, idx); });
    if (out.trace) {
      var traced = rows.filter(function (r) { return r.id === out.trace.target; })[0], iv = traced ? traced.interval : 2.5;
      out.trace.hops.forEach(function (h) { finish(h, iv, from, bucket, idx); });
      out.trace.rounds = sum(roundsIn(iv, from, bucket), 0, WT.N - 1);
      geoFill(out.trace, traced ? traced.ip : null);
    }
    return out;
  }

  function fail(code) { return Promise.reject(code); }
  var CMD = {
    snapshot: function (a) { return snapshot(a.req); },
    add_target: function (a) {
      var r = a.req || {}, host = String(r.host || '').trim();
      if (!host || /\s/.test(host)) return fail('invalid_host');
      if (/\.invalid$/.test(host)) return fail('resolve_failed');
      var have = rows.filter(function (x) { return x.host === host && !x.hopOf; })[0];
      if (have) return { id: have.id };
      var id = 'm' + (++seq), seed = host.split('').reduce(function (s, c) { return s * 31 + c.charCodeAt(0) >>> 0; }, 7);
      var base = 5 + (seed % 60);
      rows.push({ id: id, targetId: seq, host: host, name: null, ip: null, probe: r.probe || 'icmp', interval: r.intervalS || 2.5,
        max: Math.ceil(base * 1.6), series: S({ seed: seed, base: base, noise: base / 10 }), loss: [], status: 'ok', paused: false, error: null });
      recent = [host].concat(recent.filter(function (h) { return h !== host; })).slice(0, 10);
      return { id: id };
    },
    remove_target: function (a) { rows = rows.filter(function (r) { return r.id !== a.id && r.hopOf !== a.id; }); return null; },
    set_paused: function (a) { rows.forEach(function (r) { if (r.id === a.id) r.paused = !!a.paused; }); return null; },
    set_interval: function (a) { rows.forEach(function (r) { if (r.id === a.id) r.interval = a.intervalS; }); return null; },
    set_order: function (a) {
      var pos = {}; (a.ids || []).forEach(function (id, i) { pos[id] = i; });
      rows.sort(function (x, y) { return (pos[x.id] != null ? pos[x.id] : 1e9) - (pos[y.id] != null ? pos[y.id] : 1e9); });
      return null;
    },
    pin_hop: function (a) {
      var t = rows.filter(function (r) { return r.id === a.targetId; })[0];
      var h = t ? traceOf(t.host).hops.filter(function (x) { return x.ttl === a.ttl; })[0] : null;
      if (!t || !h || !h.addr) return fail('no_such_hop');
      var id = 'p' + (++seq);
      // own copy of the series: live time shifts the row and the trace hop each once
      rows.push({ id: id, targetId: t.targetId, host: h.addr, name: h.name || null, ip: h.addr, hopOf: t.id, hop: h.ttl, probe: t.probe,
        interval: t.interval, max: h.max || 10, series: h.series.slice(), loss: (h.loss || []).slice(), partial: (h.partial || []).slice(), status: h.status, paused: false, error: null });
      return { id: id };
    },
    unpin_hop: function (a) { rows = rows.filter(function (r) { return r.id !== a.id; }); return null; },
    get_settings: function () { return copy(settings); },
    set_settings: function (a) {
      var p = Object.assign({}, a.settings || {}), hm = p.home;
      // home as the core takes it: {lat, lon} rounded to 0.1 degree; null clears it; a missing key, or a point off the
      // globe, leaves it as it is
      if (hm === null) p.home = null;
      else if (onGlobe(hm)) p.home = { lat: tenth(hm.lat), lon: tenth(hm.lon) };
      else delete p.home;
      Object.assign(settings, p);
      try { sessionStorage.setItem('wt-mock-settings', JSON.stringify(settings)); } catch (e) { /* no storage */ }
      return copy(settings);
    },
    recent_inputs: function () { return recent.slice(); },
    export_report: function (a) {
      var r = a.req || {}, names = { html: 'report.html', png: 'timeline.png', csv: 'hops.csv', json: 'report.json' };
      var files = (r.formats || []).map(function (f) { return names[f]; }).filter(Boolean);
      if ((r.formats || []).indexOf('csv') >= 0) files.push('samples.csv');
      if (r.mapPngBase64 != null && isPng(r.mapPngBase64)) files.push('map.png'); // the route map, last, whatever the formats
      // As the core: a folder <name>-<time> inside the picked one (or Documents\Whilo Trace); the name is the
      // target's host when every row belongs to one target, else whilo-trace; the time is the end of the period.
      var hosts = {};
      (r.rows || []).forEach(function (id) {
        var row = rows.filter(function (x) { return x.id === id; })[0], own = row && row.hopOf ? rows.filter(function (x) { return x.id === row.hopOf; })[0] : row;
        if (own) hosts[own.targetId] = own.host;
      });
      var keys = Object.keys(hosts), name = keys.length === 1 ? hosts[keys[0]].replace(/[^A-Za-z0-9.-]/g, '_') : 'whilo-trace';
      var base = String(r.dir || '').trim() || 'C:\\Users\\you\\Documents\\Whilo Trace';
      return { dir: base.replace(/[\\/]+$/, '') + '\\' + name + '-' + fileStamp(r.toMs || END, r.tzOffsetMin || 0), files: files };
    },
    pick_folder: function () { return 'D:\\Reports'; },
    open_folder: function () { return null; },
    // Places database, in the core's order (docs/ui-contract.md, Places database) without the network: the mock's server
    // has the month of its clock out, and an offer is made once per page load (the core: the day of the last offer and
    // of the last answered check are kept, so at most one answered request goes out a day).
    geo_check: function () {
      if (settings.geoCheck === false) return { offer: null, reason: 'disabled' };
      if (GEO.asked) return { offer: null, reason: 'asked' };
      if (geoUsable() && GEO.month >= GEO_NOW) return { offer: null, reason: 'current' };
      GEO.asked = true;
      return { offer: { month: GEO_NOW, bytes: GEO_CITY }, reason: 'offer' };
    },
    // A download of about 3 s: the "geo" event at the start (total not known yet, as in the core), every 250 ms and at
    // the end; the snapshot shows it too and, once done, the new month. Flat arguments, as the core takes them.
    geo_download: function (a) {
      if (typeof a.month !== 'string') return fail(badArgs('geo_download', 'month'));
      var month = a.month;
      if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(month)) return fail('geo_invalid: not a month: ' + JSON.stringify(month));
      if (GEO.download && GEO.download.state === 'running') return fail('geo_busy');
      if (geoUsable() && month <= GEO.month) return fail('geo_invalid: ' + month + ' is not newer than ' + GEO.month);
      var t0 = Date.now();
      GEO.download = { state: 'running', month: month, received: 0, total: null, error: null };
      geoEmit();
      GEO.timer = setInterval(function () {
        var d = GEO.download;
        d.total = GEO_CITY + GEO_ASN;
        d.received = Math.min(d.total, Math.round(d.total * (Date.now() - t0) / 3000));
        if (d.received >= d.total) {
          clearInterval(GEO.timer); GEO.timer = null;
          d.state = 'done'; GEO.month = d.month; GEO.source = 'downloaded'; GEO.broken = false;
        }
        geoEmit();
      }, 250);
      return null;
    },
    geo_cancel: function () {
      var d = GEO.download;
      if (d && d.state === 'running') { clearInterval(GEO.timer); GEO.timer = null; d.state = 'cancelled'; d.error = 'geo_cancelled'; geoEmit(); }
      return null;
    },
    // Back to the database that comes with the program.
    geo_remove: function () {
      if (GEO.download && GEO.download.state === 'running') return fail('geo_busy');
      if (GEO.source === 'downloaded') { GEO.month = GEO0.month; GEO.source = GEO0.month ? 'bundled' : null; GEO.broken = GEO0.broken; }
      GEO.download = null;
      return null;
    },
    // The picture of the map, flat arguments { dir, name, pngBase64 } as the core takes them: the name cleaned as the
    // core cleans it, never over a picture saved before in this page (-2, -3, ...).
    save_image: function (a) {
      if (typeof a.name !== 'string') return fail(badArgs('save_image', 'name'));
      if (typeof a.pngBase64 !== 'string') return fail(badArgs('save_image', 'pngBase64'));
      if (!isPng(a.pngBase64)) return fail('io: not a PNG picture');
      var dir = (String(a.dir || '').trim() || 'C:\\Users\\you\\Documents\\Whilo Trace').replace(/[\\/]+$/, '');
      var stem = imageStem(a.name), path = dir + '\\' + stem + '.png';
      for (var k = 2; SAVED[path.toLowerCase()]; k++) path = dir + '\\' + stem + '-' + k + '.png';
      SAVED[path.toLowerCase()] = true;
      return { path: path };
    },
  };
  var SAVED = {};
  // A PNG picture as base64 or a data: URL, as the core reads it (the text after the last comma); anything else is not.
  function isPng(s) {
    if (typeof s !== 'string' || s.length > 64 * 1048576) return false;
    var b = s.split(',').pop().replace(/[\s=]/g, '');
    return /^[A-Za-z0-9+\/]*$/.test(b) && /^iVBORw0KGg[o-r]/.test(b);
  }
  // The file name of a picture without .png, as the core makes it: characters other than ASCII letters, digits, "-", "_"
  // and "." become "_", dots and underscores at either end go, at most 120 characters, Windows device names get "_".
  function imageStem(name) {
    var s = name.trim();
    if (/\.png$/i.test(s)) s = s.slice(0, -4);
    s = Array.from(s).slice(0, 120).map(function (c) { return /^[A-Za-z0-9._-]$/.test(c) ? c : '_'; }).join('').replace(/^[._]+|[._]+$/g, '');
    if (!s) return 'whilo-trace-map';
    return /^(CON|PRN|AUX|NUL|COM\d|LPT\d)$/.test(s.split('.')[0].toUpperCase()) ? '_' + s : s;
  }

  function shift() {
    // Time goes on: every series moves one bucket left, the new bucket repeats a recent value.
    rows.concat(TRACE.hops, TRACE_G.hops).forEach(function (r) {
      if (r.paused || !r.series) return;
      var v = r.series; v.push(v[v.length - 7]); v.shift();
      ['loss', 'partial'].forEach(function (k) { if (r[k]) r[k] = r[k].map(function (i) { return i - 1; }).filter(function (i) { return i >= 0; }); });
    });
    END += 5000;
  }

  window.MOCK = {
    call: function (cmd, args) {
      var f = CMD[cmd];
      if (window.MOCK.down) return Promise.reject('core_unavailable');
      if (!f) return Promise.reject('unknown_command: ' + cmd);
      try { return Promise.resolve(f(args || {})); } catch (e) { return Promise.reject(String(e)); }
    },
    on: function (name, cb) { (listeners[name] = listeners[name] || []).push(cb); return Promise.resolve(function () {}); },
    snapshot: snapshot,
    // ?mock=down: the core does not answer; set it to false in the console and the next request goes through.
    down: FLAGS.indexOf('down') >= 0,
  };

  if (FLAGS.indexOf('live') >= 0) {
    var tick = 0;
    setInterval(function () {
      if (window.MOCK.down) return; // a core that does not answer sends no rounds either
      if (++tick % 2 === 0) shift();
      // rounds of 1.1.1.1 and of 8.8.8.8 (the google row: the route map's demo route)
      [1, 2].forEach(function (tid) { (listeners.round || []).forEach(function (cb) { cb(tid); }); });
    }, 2500);
  }

  // Parts of the mockups mount right after their scripts load, so the first snapshot is applied at once
  // (not when the core is down: then WT stays empty, as in the app).
  if (!window.MOCK.down) WT.apply(snapshot({ window: settings.window }));
})();
