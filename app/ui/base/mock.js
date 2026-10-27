// Browser stand-in for the Rust core: answers the same commands as the app (docs/ui-contract.md)
// with the mockup scenario, so every part can be built and checked in a plain browser.
// Not used inside the app (window.__TAURI__ is there). Address flags, comma separated: ?mock=empty (first run),
// ?mock=live (a round event every 2.5 s, data shifts as if time goes on), ?mock=strict (no trace on the main
// screen, exactly like the app), ?mock=down (the core does not answer: every command fails with
// core_unavailable until MOCK.down = false is set in the console).
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
    { id: 'google', targetId: 2, host: '8.8.8.8', name: 'dns.google', ip: '8.8.8.8', probe: 'icmp', interval: 2.5, max: 20,
      series: S({ seed: 23, base: 9.4, noise: 1.2, spikes: [[30, 16.5], [95, 14.2]] }), loss: [16, 43, 44, 70, 97, 109], status: 'loss' },
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

  // Short path summary that every row carries (the core sends it with each target):
  // route length, where the loss starts, hops silent for this probe type, hops that answer only some probes.
  function pathOf(probe) {
    var o = TRACE.hops.filter(function (h) { return h.origin; })[0];
    return {
      hops: TRACE.hops.length,
      origin: o ? { ttl: o.ttl, addr: o.addr } : null,
      silent: TRACE.hops.filter(function (h) { return (h.byProbe[probe] || h.status) === 'silent'; }).map(function (h) { return h.ttl; }),
      partial: TRACE.hops.filter(function (h) { return (h.byProbe[probe] || h.status) === 'partial'; }).map(function (h) {
        return { ttl: h.ttl, addr: h.addr, name: h.name || null, noReplyPct: (h.partial || []).length / WT.N * 100 };
      }),
    };
  }
  rows.forEach(function (r) { if (!r.hopOf) r.path = pathOf(r.probe); });

  var settings = { warnMs: 100, badMs: 200, theme: 'system', lang: 'ru', window: '10m' };
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
  // A hop that never answers stays silent or without reply; one that answers only some probes stays partial
  // while the period has such buckets.
  function rangeStatus(t, s) {
    if (t.status === 'silent' || t.status === 'noreply') return t.status;
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

  function snapshot(req) {
    req = req || {};
    var win = req.window || settings.window || '10m', span = WT.windowSeconds[win] * 1000;
    var to = req.endMs || END, from = to - span, bucket = span / WT.N;
    var out = {
      N: WT.N, nowMs: END, fromMs: from, toMs: to, live: !req.endMs, bucketMs: bucket,
      targets: copy(rows),
      // The core sends the trace only for req.trace. The mockups also read it on the main screen,
      // so the mock keeps it there unless ?mock=strict (strict is how the app behaves).
      trace: req.trace ? (rows.some(function (r) { return r.id === req.trace && r.host === '1.1.1.1'; }) ? copy(TRACE) : null)
        : (FLAGS.indexOf('strict') >= 0 ? null : copy(TRACE)),
      history: { fromMs: to - 86400000, toMs: to, slots: 200, loss: HISTORY.slice() },
      loss: LOSS.slice(), ticks: TICKS, settings: copy(settings), probesAvailable: ['icmp'],
      ratePps: rows.filter(function (r) { return !r.paused; }).length ? 18 : 0, recent: recent.slice(),
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
      var t = rows.filter(function (r) { return r.id === a.targetId; })[0], h = TRACE.hops.filter(function (x) { return x.ttl === a.ttl; })[0];
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
      Object.assign(settings, a.settings || {});
      try { sessionStorage.setItem('wt-mock-settings', JSON.stringify(settings)); } catch (e) { /* no storage */ }
      return copy(settings);
    },
    recent_inputs: function () { return recent.slice(); },
    export_report: function (a) {
      var r = a.req || {}, names = { html: 'report.html', png: 'timeline.png', csv: 'hops.csv', json: 'report.json' };
      var files = (r.formats || []).map(function (f) { return names[f]; }).filter(Boolean);
      if ((r.formats || []).indexOf('csv') >= 0) files.push('samples.csv');
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
  };

  function shift() {
    // Time goes on: every series moves one bucket left, the new bucket repeats a recent value.
    rows.concat(TRACE.hops).forEach(function (r) {
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
      (listeners.round || []).forEach(function (cb) { cb(1); });
    }, 2500);
  }

  // Parts of the mockups mount right after their scripts load, so the first snapshot is applied at once
  // (not when the core is down: then WT stays empty, as in the app).
  if (!window.MOCK.down) WT.apply(snapshot({ window: settings.window }));
})();
