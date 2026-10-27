// WT: shared dictionaries, chart path builders and the data snapshot that every part reads.
// The data comes from base/live.js: the Rust core in the app, base/mock.js in a plain browser.
// Both fill WT through WT.apply(snapshot); the snapshot format is described in docs/ui-contract.md.
(function () {
  function rng(seed) {
    var s = seed >>> 0;
    return function () {
      s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
      return s / 4294967296;
    };
  }
  function N() { return WT.N; }

  // A series is a list of N bucket values in ms (null: no reply in the bucket).
  // The mock may still describe one by generator parameters; real data is always a list.
  function series(p) {
    if (Array.isArray(p)) return p;
    var r = rng(p.seed), v = [];
    for (var i = 0; i < N(); i++) v.push(Math.max(0.1, p.base + (r() - 0.5) * p.noise));
    (p.spikes || []).forEach(function (s) { v[s[0]] = s[1]; });
    return v;
  }
  function sample(seed, share) {
    var r = rng(seed), out = [];
    for (var i = 0; i < N(); i++) if (r() < share) out.push(i);
    return out;
  }

  // Probe types. The engine sends ICMP only for now; WT.probesAvailable says which ones work.
  var probes = {
    icmp: { label: 'ICMP', ru: 'ICMP', en: 'ICMP' },
    tcp443: { label: 'TCP 443', ru: 'TCP 443', en: 'TCP 443' },
    udp: { label: 'UDP', ru: 'UDP', en: 'UDP' },
  };

  // Neutral status names; the explanation lives only in the hint (tooltip). {probe} is the probe type.
  // hint_target_* replaces the hint at the target itself, where there are no hops after it.
  var statuses = {
    ok: { ru: 'в норме', en: 'OK', hint_ru: 'Отвечает на все пробы, потерь нет.', hint_en: 'Answers every probe, no loss.' },
    loss: { ru: 'потери', en: 'Loss', hint_ru: 'Пробы теряются, и потери продолжаются на хопах дальше по пути.', hint_en: 'Probes are lost and the loss carries on to later hops.',
      hint_target_ru: 'Часть проб до цели теряется.', hint_target_en: 'Some probes to the target are lost.' },
    partial: { ru: 'отвечает не на все пробы', en: 'Answers some probes',
      hint_ru: 'Узел сам отвечает не на все пробы, а хопы дальше отвечают без потерь. Обычно так узел ограничивает свои ответы.',
      hint_en: 'The node answers only some probes while later hops answer fine. Usually the node limits its own replies.' },
    silent: { ru: 'не отвечает на {probe}', en: 'No reply to {probe}',
      hint_ru: 'Узел не отвечает на пробы этого типа, но хопы дальше отвечают: трафик через него проходит.',
      hint_en: 'The node does not answer probes of this type, but later hops do: traffic passes through it.' },
    noreply: { ru: 'нет ответа', en: 'No reply', hint_ru: 'Не отвечает ни этот узел, ни узлы дальше.', hint_en: 'Neither this node nor anything after it answers.',
      hint_target_ru: 'Цель не отвечает на пробы.', hint_target_en: 'The target does not answer the probes.' },
    origin: { ru: 'потери начинаются здесь', en: 'Loss starts here' },
  };

  var windows = [['5m', '5 мин', '5 min'], ['10m', '10 мин', '10 min'], ['30m', '30 мин', '30 min'], ['1h', '1 ч', '1 h'], ['6h', '6 ч', '6 h'], ['24h', '24 ч', '24 h']];
  var WINDOW_S = { '5m': 300, '10m': 600, '30m': 1800, '1h': 3600, '6h': 21600, '24h': 86400 };
  var intervals = [0.5, 1, 2.5, 5, 10, 30, 60]; // seconds, default 2.5

  // Time axis labels for every window, six per window, ending at endMs (local time).
  // Same format as the mockups: «HH:MM», and «вчера HH:MM» for the previous day (parts translate «вчера»).
  function pad(n) { return (n < 10 ? '0' : '') + n; }
  function ticksAt(endMs) {
    var out = {}, end = new Date(endMs);
    windows.forEach(function (w) {
      var span = WINDOW_S[w[0]] * 1000, list = [];
      for (var k = 0; k < 6; k++) {
        var d = new Date(endMs - span + (span * k) / 5);
        var t = pad(d.getHours()) + ':' + pad(d.getMinutes());
        if (d.toDateString() !== end.toDateString()) t = 'вчера ' + t;
        list.push(t);
      }
      out[w[0]] = list;
    });
    return out;
  }

  // SVG path builders. Charts use viewBox="0 0 W H", preserveAspectRatio="none" and
  // vector-effect: non-scaling-stroke, so they stretch to any width with crisp lines.
  function X(i, w) { return (i / Math.max(1, N() - 1)) * w; }
  function line(v, max, gaps, w, h, pad) {
    w = w || 1000; h = h || 120; pad = pad || 0;
    var set = toSet(gaps), d = '', pen = false;
    for (var i = 0; i < N(); i++) {
      if (set[i] || v[i] == null) { pen = false; continue; }
      var y = h - pad - (Math.min(v[i], max) / max) * (h - 2 * pad);
      d += (pen ? 'L' : 'M') + X(i, w).toFixed(1) + ' ' + y.toFixed(1) + ' ';
      pen = true;
    }
    return d || 'M0 0';
  }
  function bars(idx, top, height, w) {
    w = w || 1000;
    var bw = w / N();
    var d = (idx || []).map(function (i) {
      var a = Math.max(0, X(i, w) - bw / 2).toFixed(1), b = Math.min(w, X(i, w) + bw / 2).toFixed(1);
      return 'M' + a + ' ' + top + 'H' + b + 'V' + (top + height) + 'H' + a + 'Z';
    }).join('');
    return d || 'M0 0';
  }
  function zone(lo, hi, max, w, h) {
    w = w || 1000; h = h || 120;
    if (max <= lo) return 'M0 0';
    var y = function (val) { return (h - (Math.min(val, max) / max) * h).toFixed(1); };
    return 'M0 ' + y(Math.min(hi, max)) + 'H' + w + 'V' + y(lo) + 'H0Z';
  }
  function jitter(v, gaps, w, h) {
    w = w || 1000; h = h || 16;
    var set = toSet(gaps), js = [];
    for (var i = 1; i < N(); i++) js.push(set[i] || set[i - 1] || v[i] == null || v[i - 1] == null ? null : Math.abs(v[i] - v[i - 1]));
    var known = js.filter(function (j) { return j !== null; });
    if (!known.length) return 'M0 0';
    var top = Math.max.apply(null, [0.001].concat(known)), d = '', pen = false;
    js.forEach(function (j, k) {
      if (j === null) { pen = false; return; }
      d += (pen ? 'L' : 'M') + X(k + 1, w).toFixed(1) + ' ' + (h - 1 - (j / top) * (h - 3)).toFixed(1) + ' ';
      pen = true;
    });
    return d || 'M0 0';
  }
  function hatch(w, h, step) {
    w = w || 1000; h = h || 120; step = step || 16;
    var d = '';
    for (var x = -h; x < w; x += step) d += 'M' + x + ' ' + h + 'L' + (x + h) + ' 0';
    return d;
  }
  function toSet(list) { var s = {}; (list || []).forEach(function (i) { s[i] = true; }); return s; }

  // Numbers in Russian style: decimal comma, whole numbers from 10 up.
  function fmt(n) { return n >= 10 ? String(Math.round(n)) : n.toFixed(1).replace('.', ','); }

  // Window statistics for a target or a hop: loss %, avg, min, max, last, jitter.
  // The Rust core sends them ready in t.stats; the formula below is for the mock and for old callers.
  function stats(t) {
    var v = series(t.series), gapList = (t.loss || []).concat(t.partial || []);
    if (t.stats) return Object.assign({ values: v, gaps: gapList }, t.stats);
    var gaps = toSet(gapList), n = N();
    var seen = v.filter(function (x, i) { return !gaps[i] && x != null; });
    var sum = seen.reduce(function (a, b) { return a + b; }, 0), jit = 0;
    for (var i = 1; i < seen.length; i++) jit += Math.abs(seen[i] - seen[i - 1]);
    var last = null;
    for (var k = n - 1; k >= 0; k--) { if (!gaps[k] && v[k] != null) { last = v[k]; break; } }
    return {
      values: v,
      gaps: gapList,
      lossPct: ((t.loss || []).length / n) * 100,
      noReplyPct: ((t.partial || []).length / n) * 100,
      avg: seen.length ? sum / seen.length : null,
      min: seen.length ? Math.min.apply(null, seen) : null,
      max: seen.length ? Math.max.apply(null, seen) : null,
      last: last, lastLost: !!gaps[n - 1], jitter: jit / Math.max(1, seen.length - 1),
    };
  }

  // Loads a snapshot (docs/ui-contract.md) into WT. Parts keep reading the same fields as in the mockups.
  function apply(s) {
    WT.N = s.N;
    WT.nowMs = s.nowMs; WT.fromMs = s.fromMs; WT.toMs = s.toMs; WT.live = s.live; WT.bucketMs = s.bucketMs;
    WT.targets = s.targets || [];
    WT.trace = s.trace || null;
    WT.history = s.history || { fromMs: 0, toMs: 0, slots: 200, loss: [] };
    WT.history24h = WT.history.loss;
    WT.LOSS = s.loss || [];
    WT.ticks = s.ticks || ticksAt(s.toMs);
    WT.settings = s.settings || WT.settings;
    WT.probesAvailable = s.probesAvailable || ['icmp'];
    WT.ratePps = s.ratePps || 0;
    WT.recent = s.recent || [];
  }

  var WT = window.WT = {
    N: 120, rng: rng, series: series, sample: sample, LOSS: [],
    probes: probes, targets: [], trace: null, statuses: statuses,
    ticks: {}, windows: windows, windowSeconds: WINDOW_S, intervals: intervals, history24h: [], history: null,
    nowMs: 0, fromMs: 0, toMs: 0, live: true, bucketMs: 5000, ratePps: 0, recent: [],
    settings: { warnMs: 100, badMs: 200, theme: 'system', lang: 'ru', window: '10m' },
    probesAvailable: ['icmp'],
    path: { line: line, bars: bars, zone: zone, jitter: jitter, hatch: hatch },
    stats: stats, fmt: fmt, apply: apply, ticksAt: ticksAt,
  };
})();
