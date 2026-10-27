// Whilo Trace, part "target trace": header, findings, hop table and the timeline of the selected hop.
// Data comes only from window.WT (base/wt.js, filled from the core snapshot): the row in WT.targets, the hops
// in WT.trace.hops, numbers in stats / rangeStats. Texts are written in the interface language (?lang=en,
// base/theme.js) and duplicated in data-en, as in the other parts.
// TR.mount(el, o) draws the screen; view.update() redraws it from WT in place (morph below), so live data keeps
// hover, the open menu, dragging, the picked period, scroll and focus. Actions leave through the o.on* callbacks.
(function () {
  'use strict';
  var EN = document.documentElement.dataset.lang === 'en';

  // ---------- text and numbers ----------
  function L(ru, en) { return EN ? en : ru; }
  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; });
  }
  function S(ru, en, cls) {
    return '<span' + (cls ? ' class="' + cls + '"' : '') + ' data-en="' + esc(en) + '">' + esc(L(ru, en)) + '</span>';
  }
  function P(pair, cls) { return S(pair.ru, pair.en, cls); }
  function nf(n, lang) {
    if (n === 0) return '0';
    var s = WT.fmt(n);
    return lang === 'en' ? s.replace(',', '.') : s;
  }
  // Axis labels and intervals: whole numbers without a decimal (5, not 5,0).
  function ni(n, lang) { return Number.isInteger(n) ? String(n) : nf(n, lang); }
  function nip(n) { return { ru: ni(n, 'ru'), en: ni(n, 'en') }; }
  // Russian keeps the unit on the number's line: a no-break space before it.
  function pr(n) { return { ru: nf(n, 'ru') + ' %', en: nf(n, 'en') + '%' }; }
  function msr(n) { return { ru: nf(n, 'ru') + ' мс', en: nf(n, 'en') + ' ms' }; }
  function sec(n) { return { ru: ni(n, 'ru') + ' с', en: ni(n, 'en') + ' s' }; }
  function msi(n) { return { ru: ni(n, 'ru') + ' мс', en: ni(n, 'en') + ' ms' }; }
  function winP(key) {
    var w = WT.windows.filter(function (x) { return x[0] === key; })[0] || WT.windows[1];
    return { ru: w[1], en: w[2] };
  }
  // Time labels of the window that ends at WT.toMs, six per window (WT.ticksAt).
  function ticksP(key) {
    var all = WT.toMs ? WT.ticksAt(WT.toMs) : WT.ticks;
    return (all[key] || all['10m'] || []).map(function (t) { return { ru: t, en: t.replace('вчера', 'yesterday') }; });
  }
  function rangeP(key) {
    var t = ticksP(key);
    if (!t.length) return { ru: '', en: '' };
    return { ru: t[0].ru + '-' + t[t.length - 1].ru, en: t[0].en + '-' + t[t.length - 1].en };
  }
  function probeP(k) { var p = WT.probes[k]; return p ? { ru: p.ru, en: p.en } : { ru: String(k).toUpperCase(), en: String(k).toUpperCase() }; }
  function probeLabel(k) { return WT.probes[k] ? WT.probes[k].label : String(k).toUpperCase(); }
  // Neutral statuses from WT.statuses; the probe type is filled in, never fixed.
  // At the destination (dest) the hint speaks of the target: there are no hops after it.
  function stP(key, probe, dest) {
    var s = WT.statuses[key] || WT.statuses.noreply, p = probeP(probe), tg = dest && s.hint_target_ru;
    return {
      ru: s.ru.replace('{probe}', p.ru), en: s.en.replace('{probe}', p.en),
      hru: ((tg ? s.hint_target_ru : s.hint_ru) || '').replace('{probe}', p.ru),
      hen: ((tg ? s.hint_target_en : s.hint_en) || '').replace('{probe}', p.en),
    };
  }
  // Node name from the data; «роутер» is the gateway label of the mock, it has an English form.
  function nameP(name) {
    if (!name) return null;
    if (name === 'роутер') return { ru: 'роутер', en: 'router' };
    return { ru: name, en: name };
  }
  function nice(v) {
    if (!(v > 0)) return 1;
    var p = Math.pow(10, Math.floor(Math.log(v) / Math.LN10)), f = v / p;
    return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * p;
  }
  function isIP(s) { return /^\d{1,3}(\.\d{1,3}){3}$/.test(s || ''); }
  function fin(v) { return v != null && isFinite(v); }
  function setOf(list) { var s = {}; (list || []).forEach(function (i) { s[i] = true; }); return s; }
  function roundsP(n) {
    var d = n % 10, dd = n % 100;
    var ru = d === 1 && dd !== 11 ? 'раунд' : d >= 2 && d <= 4 && (dd < 12 || dd > 14) ? 'раунда' : 'раундов';
    return { ru: n + ' ' + ru, en: n + (n === 1 ? ' round' : ' rounds') };
  }

  // ---------- time: local clock of the snapshot buckets ----------
  function two(x) { return (x < 10 ? '0' : '') + x; }
  function sameDay(a, b) { return new Date(a).toDateString() === new Date(b).toDateString(); }
  function dayStart(ms) { var d = new Date(ms); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); }
  // Clock time of a moment: today only the time, the day before «вчера» / «yesterday», earlier days with the
  // date in the format of the history part (he.js dateOf): «25.10.2026 08:44», «2026‑10‑25 08:44».
  function clock(ms, secs, bare) {
    var d = new Date(ms), t = two(d.getHours()) + ':' + two(d.getMinutes()) + (secs ? ':' + two(d.getSeconds()) : '');
    var ago = Math.round((dayStart(WT.nowMs || WT.toMs || ms) - dayStart(ms)) / 864e5);
    if (bare || ago <= 0) return { ru: t, en: t };
    if (ago === 1) return { ru: 'вчера ' + t, en: 'yesterday ' + t };
    return {
      ru: two(d.getDate()) + '.' + two(d.getMonth() + 1) + '.' + d.getFullYear() + ' ' + t,
      en: d.getFullYear() + '‑' + two(d.getMonth() + 1) + '‑' + two(d.getDate()) + ' ' + t,
    };
  }
  function bAt(i) { return WT.fromMs + i * WT.bucketMs; } // start of bucket i
  function tAt(i) { return clock(bAt(i), true); }
  // A picked period [fromMs, toMs): «15:54:00-15:56:00».
  function periodP(r) {
    var a = clock(r[0], true), b = clock(r[1], true, sameDay(r[0], r[1]));
    return { ru: a.ru + '-' + b.ru, en: a.en + '-' + b.en };
  }
  function durP(ms) {
    var s = Math.round(ms / 1000), h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60), x = s % 60, ru = [], en = [];
    if (h) { ru.push(h + ' ч'); en.push(h + ' h'); }
    if (m) { ru.push(m + ' мин'); en.push(m + ' min'); }
    if (x) { ru.push(x + ' с'); en.push(x + ' s'); }
    return { ru: ru.join(' ') || '0 с', en: en.join(' ') || '0 s' };
  }
  function clampI(i) { return Math.max(0, Math.min(WT.N - 1, i)); }
  // A picked period as bucket indices of the current window, clamped; null when it is out of view.
  function rangeIdx(r) {
    if (!r || !(WT.bucketMs > 0) || !(WT.N > 0)) return null;
    var a = Math.floor((r[0] - WT.fromMs) / WT.bucketMs), b = Math.ceil((r[1] - WT.fromMs) / WT.bucketMs) - 1;
    if (b < 0 || a > WT.N - 1 || b < a) return null;
    return [Math.max(0, a), Math.min(WT.N - 1, b)];
  }
  function sameRange(a, b) { return !!a && !!b && a[0] === b[0] && a[1] === b[1]; }

  // ---------- icons: own 1.5 px line, stroke currentColor ----------
  var A = ' fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"';
  var ICON = {
    back: '<svg class="ico" width="16" height="16" viewBox="0 0 16 16"' + A + '><path d="M13 8H3.5M7.5 4 3.5 8l4 4"/></svg>',
    down: '<svg class="ico" width="12" height="12" viewBox="0 0 12 12"' + A + '><path d="M3 4.5 6 7.5l3-3"/></svg>',
    pause: '<svg class="ico" width="16" height="16" viewBox="0 0 16 16"' + A + '><path d="M6 4v8M10 4v8"/></svg>',
    play: '<svg class="ico" width="16" height="16" viewBox="0 0 16 16"' + A + '><path d="M5.5 3.8v8.4L12 8z"/></svg>',
    report: '<svg class="ico" width="16" height="16" viewBox="0 0 16 16"' + A + '><path d="M4 2.5h5.5L12 5v8.5H4z"/><path d="M9.5 2.5V5H12M6.2 8h3.6M6.2 10.5h3.6"/></svg>',
    pin: '<svg class="ico" width="16" height="16" viewBox="0 0 16 16"' + A + '><rect x="2.5" y="3.5" width="11" height="9" rx="1.5"/><path d="M8 6v4M6 8h4"/></svg>',
    pinned: '<svg class="ico" width="16" height="16" viewBox="0 0 16 16"' + A + '><rect x="2.5" y="3.5" width="11" height="9" rx="1.5"/><path d="M5.8 8.1l1.5 1.5 3-3.1"/></svg>',
    check: '<svg class="ico" width="14" height="14" viewBox="0 0 14 14"' + A + '><path d="M3 7.2l2.6 2.6L11 4.4"/></svg>',
    range: '<svg class="ico" width="16" height="16" viewBox="0 0 18 18"' + A + '><path d="M3 3v12M15 3v12M3 9h12M6 6.5 3.5 9 6 11.5M12 6.5 14.5 9 12 11.5"/></svg>',
  };

  // ---------- picked period on the charts ----------
  function X1000(i) { return i / Math.max(1, WT.N - 1) * 1000; }
  function rangePath(a, b, H) {
    var bw = 1000 / WT.N, x0 = Math.max(0, X1000(a) - bw / 2), x1 = Math.min(1000, X1000(b) + bw / 2);
    return 'M' + x0.toFixed(1) + ' 0H' + x1.toFixed(1) + 'V' + H + 'H' + x0.toFixed(1) + 'Z';
  }
  function edgePath(a, b, H) {
    var bw = 1000 / WT.N, x0 = Math.max(0, X1000(a) - bw / 2), x1 = Math.min(1000, X1000(b) + bw / 2);
    return 'M' + x0.toFixed(1) + ' 0V' + H + 'M' + x1.toFixed(1) + ' 0V' + H;
  }
  function selPaths(sel, H) { return '<path class="p-sel" d="' + (sel ? rangePath(sel[0], sel[1], H) : 'M0 0') + '"/>'; }
  function selEdge(sel, H) { return '<path class="p-sel-edge" d="' + (sel ? edgePath(sel[0], sel[1], H) : 'M0 0') + '"/>'; }

  // Hatch "does not answer probes of this type": one pattern per page.
  function ensureDefs() {
    if (document.getElementById('trHatch')) return;
    var d = document.createElement('div');
    d.innerHTML = '<svg width="0" height="0" style="position:absolute" aria-hidden="true" focusable="false"><defs>' +
      '<pattern id="trHatch" width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">' +
      '<rect width="1.5" height="4" class="hatch-fill"/></pattern>' +
      '<pattern id="trHatchL" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">' +
      '<rect width="1" height="6" class="hatch-fill"/></pattern></defs></svg>';
    document.body.insertBefore(d.firstChild, document.body.firstChild);
  }

  // ---------- data ----------
  function byId(id) {
    for (var i = 0; i < WT.targets.length; i++) if (WT.targets[i].id === id) return WT.targets[i];
    return null;
  }
  function sameTarget(a, b) { return a.targetId != null && b.targetId != null ? a.targetId === b.targetId : a.host === b.host; }
  // Probe types of a target: one row per type in WT.targets, pinned hops aside.
  function probeRows(row) { return WT.targets.filter(function (t) { return !t.hopOf && sameTarget(t, row); }); }
  // Hops of the target pinned to the main screen from any of its probe rows: ttl -> pinned row id.
  // A hop pinned from the shown row keeps that row, so its button takes off exactly what it shows.
  function pinsOf(row) {
    var own = {}, p = {};
    own[row.id] = true;
    probeRows(row).forEach(function (r) { own[r.id] = true; });
    WT.targets.forEach(function (t) {
      if (t.hopOf && own[t.hopOf] && t.hop != null && (!p[t.hop] || t.hopOf === row.id)) p[t.hop] = t.id;
    });
    return p;
  }
  // Status for a picked period: the core's (rangeStats.status), otherwise its rule for loss and OK
  // (loss from LOSS_NOTABLE_PCT, 2 %); the other statuses stay. Without a period the status is the core's.
  var LOSS_NOTABLE_PCT = 2;
  function periodStatus(status, s) {
    if (s.status) return s.status;
    if (status !== 'loss' && status !== 'ok') return status;
    return s.lossPct >= LOSS_NOTABLE_PCT ? 'loss' : 'ok';
  }
  // Numbers for the picked period by the same formula as WT.stats: loss and "no reply" stay out of the latency.
  // Used until the core answers with rangeStats for this period.
  function statsR(src, idx) {
    var v = WT.series(src.series), loss = setOf(src.loss), part = setOf(src.partial), seen = [], lost = 0, np = 0, n = 0;
    for (var i = idx[0]; i <= idx[1]; i++) {
      n++;
      if (loss[i]) lost++; else if (part[i]) np++; else if (v[i] != null) seen.push(v[i]);
    }
    var sum = seen.reduce(function (a, b) { return a + b; }, 0), jit = 0;
    for (var k = 1; k < seen.length; k++) jit += Math.abs(seen[k] - seen[k - 1]);
    return {
      values: v, lossPct: n ? lost / n * 100 : 0, noReplyPct: n ? np / n * 100 : 0,
      avg: seen.length ? sum / seen.length : null, min: seen.length ? Math.min.apply(null, seen) : null, max: seen.length ? Math.max.apply(null, seen) : null,
      last: seen.length ? seen[seen.length - 1] : null, jitter: seen.length > 1 ? jit / (seen.length - 1) : null, sent: null,
    };
  }
  // Window numbers (t.stats from the core) or the picked period (t.rangeStats once the snapshot is for it).
  function statsOf(src, idx, rs) {
    if (!idx) return WT.stats(src);
    if (rs && src.rangeStats) return Object.assign({ values: WT.series(src.series) }, src.rangeStats);
    return statsR(src, idx);
  }
  // When a row stopped: the end of its last bucket with data; null when the whole window is empty.
  function pauseTime(t) {
    var v = WT.series(t.series || []), loss = setOf(t.loss);
    for (var i = WT.N - 1; i >= 0; i--) if (v[i] != null || loss[i]) return i < WT.N - 1 ? bAt(i + 1) : WT.nowMs || WT.toMs;
    return null;
  }

  // ---------- screen model ----------
  // st: id (row id of the traced target and probe type), win, range ([fromMs, toMs) or null), dataRange (the period
  // the snapshot in WT was made for), selected (ttl), loading (no snapshot for this target yet), pausedAt,
  // down (no snapshot comes from the core), note (text of the last failed action).
  function model(st) {
    var row = st.id ? byId(st.id) : null;
    if (!row) return { kind: st.down ? 'down' : st.id ? 'missing' : 'none' };
    var rows = probeRows(row);
    if (rows.indexOf(row) < 0) rows = [row].concat(rows);
    var m = {
      target: row, probeRows: rows, probes: rows.map(function (p) { return p.probe; }), probe: row.probe,
      win: st.win, paused: !!row.paused, interval: row.interval, pausedAt: st.pausedAt && st.pausedAt.id === row.id ? st.pausedAt.at : null,
    };
    var tr = WT.trace, traceRow = tr ? byId(tr.target) : null;
    if (tr && traceRow && !sameTarget(traceRow, row)) tr = null; // a trace left from another target
    if (st.loading) { m.kind = st.down ? 'down' : 'loading'; return m; }
    if (!tr || !tr.hops || !tr.hops.length || (tr.rounds != null && tr.rounds <= 0)) { m.kind = 'wait'; return m; }

    // The core traces the row asked for. The mock answers with its ICMP trace for every type: then the other
    // type shows hop statuses from byProbe and numbers only for the destination (its own row).
    var main = !traceRow || traceRow.id === row.id || traceRow.probe === row.probe;
    var idx = rangeIdx(st.range), rs = sameRange(st.range, st.dataRange), pins = pinsOf(row);
    var hops = tr.hops.map(function (hp) {
      var status = main ? hp.status : (hp.byProbe && hp.byProbe[row.probe]) || 'noreply';
      var src = null;
      if (main) { if (status !== 'silent' && hp.series) src = hp; } else if (hp.dest) src = row;
      var s = src ? statsOf(src, idx, rs) : null;
      if (idx && s) status = periodStatus(status, s);
      return {
        ttl: hp.ttl, addr: hp.addr || null, name: hp.name || null, dest: !!hp.dest, status: status,
        src: src, s: s, full: src ? WT.stats(src) : null, by: hp.byProbe || {}, pin: pins[hp.ttl] || null, pinned: !!pins[hp.ttl], origin: false,
      };
    });
    // Where the loss starts: the core marks it for the window; for a picked period the engine rule runs here:
    // from the destination back while the loss carries on, silent hops do not count.
    if (main && !idx && tr.hops.some(function (hp) { return 'origin' in hp; })) {
      hops.forEach(function (h, i) { h.origin = !!tr.hops[i].origin; });
    } else {
      var di = hops.length - 1;
      hops.forEach(function (h, i) { if (h.dest) di = i; });
      if (hops[di] && hops[di].status === 'loss') {
        var oi = di;
        for (var j = di - 1; j >= 0; j--) {
          if (hops[j].status === 'loss') oi = j;
          else if (hops[j].status !== 'silent') break;
        }
        hops[oi].origin = true;
      }
    }
    // Scale of the latency band over the whole window: it does not jump when a period is picked.
    var top = 0;
    hops.forEach(function (h) { if (h.full && fin(h.full.max)) top = Math.max(top, h.full.max); });
    var dest = hops.filter(function (h) { return h.dest; })[0] || null;
    var lead = dest && dest.s ? dest.s : statsOf(row, idx, rs);
    // Rounds: in the window from the trace; in a picked period from its numbers, or by its length until they come.
    var rounds = idx
      ? (lead && lead.sent != null ? lead.sent : Math.round((st.range[1] - st.range[0]) / 1000 / (row.interval > 0 ? row.interval : 2.5)))
      : tr.rounds != null ? tr.rounds : (lead && lead.sent) || 0;
    var sel = st.selected;
    if (!hops.some(function (h) { return h.ttl === sel; })) {
      var o = hops.filter(function (h) { return h.origin; })[0];
      sel = (o || dest || hops[hops.length - 1]).ttl;
    }
    m.kind = 'trace'; m.main = main; m.hops = hops; m.dest = dest; m.lead = lead; m.scale = nice(top);
    m.rounds = rounds; m.range = idx ? st.range : null; m.idx = idx; m.selected = sel;
    return m;
  }
  function hopOf(m, ttl) { return m.hops.filter(function (h) { return h.ttl === ttl; })[0]; }
  function topOf(src) { return src.max > 0 ? src.max : nice(WT.stats(src).max || 1); }
  function warnMs() { return (WT.settings && WT.settings.warnMs) || 100; }
  function badMs() { return (WT.settings && WT.settings.badMs) || 200; }

  // ---------- header ----------
  function header(m, st) {
    var h = '<header class="tr-head">';
    h += '<button class="btn btn-ghost tr-back" data-act="back">' + ICON.back + S('Все цели', 'All targets') + '</button>';
    var t = m.target;
    if (!t) return h + '</header>';
    // Under the host: the reverse name of an address, the address of a name.
    var sub = isIP(t.host) ? (t.name && t.name !== t.host ? t.name : '') : (t.ip || (t.name && t.name !== t.host ? t.name : ''));
    // A long name ends in an ellipsis; the full one is in the title.
    h += '<div class="tr-ident"><h1' + (isIP(t.host) ? ' class="num"' : '') + ' title="' + esc(t.host) + '">' + esc(t.host) + '</h1>';
    if (sub) h += '<span class="sub' + (isIP(sub) ? ' mono' : '') + '" title="' + esc(sub) + '">' + esc(sub) + '</span>';
    if (m.probes.length > 1) {
      h += '<div class="seg tr-probe-seg" role="group" aria-label="' + esc(L('Тип пробы', 'Probe type')) + '">' +
        m.probes.map(function (k) {
          return '<button data-probe="' + esc(k) + '" aria-pressed="' + (k === m.probe) + '">' + P(probeP(k)) + '</button>';
        }).join('') + '</div>';
    } else {
      h += '<span class="chip" title="' + esc(L('Тип пробы', 'Probe type')) + '">' + P(probeP(m.probe)) + '</span>';
    }
    if (m.paused) {
      var pa = m.pausedAt ? clock(m.pausedAt, false) : null;
      h += '<span class="chip chip-pause">' + (pa ? S('пауза с ' + pa.ru, 'paused at ' + pa.en) : S('пауза', 'paused')) + '</span>';
    }
    h += '</div>';

    h += '<div class="tr-set">';
    h += '<div class="tr-field">' + S('Интервал', 'Interval', 'lbl') +
      '<button class="btn tr-dd" data-act="interval" aria-haspopup="listbox" aria-expanded="' + !!st.menu + '">' + P(sec(m.interval)) + ICON.down + '</button>';
    if (st.menu) {
      h += '<div class="tr-menu" role="listbox" aria-label="' + esc(L('Интервал', 'Interval')) + '">' +
        WT.intervals.map(function (v) {
          var cur = v === m.interval;
          return '<button role="option" data-iv="' + v + '" aria-checked="' + cur + '">' + P(sec(v)) +
            (v === 2.5 ? S('по умолчанию', 'default', 'def') : '<span></span>') + (cur ? ICON.check : '<span></span>') + '</button>';
        }).join('') + '</div>';
    }
    h += '</div>';
    h += '<div class="tr-field">' + S('Окно', 'Window', 'lbl') + '<div class="seg" role="group" aria-label="' + esc(L('Окно времени', 'Time window')) + '">' +
      WT.windows.map(function (w) {
        return '<button data-win="' + w[0] + '" aria-pressed="' + (w[0] === m.win) + '">' + S(w[1], w[2]) + '</button>';
      }).join('') + '</div></div>';
    h += '</div>';

    h += '<div class="tr-act">';
    h += m.paused
      ? '<button class="btn" data-act="pause">' + ICON.play + S('Продолжить', 'Resume') + '</button>'
      : '<button class="btn" data-act="pause">' + ICON.pause + S('Пауза', 'Pause') + '</button>';
    var ready = m.kind === 'trace';
    h += '<button class="btn btn-primary" data-act="report"' + (ready ? '' : ' disabled') + ' title="' +
      esc(ready ? L('Откроет диалог отчета', 'Opens the report dialog') : L('Отчет будет доступен, когда появятся данные трассы', 'The report is available once there is trace data')) + '">' +
      ICON.report + S('Отчет для провайдера', 'Report for ISP') + '</button>';
    h += '</div></header>';
    return h;
  }

  // ---------- findings in plain text ----------
  function HL(ttl, ru, en) {
    return '<button class="hl-btn" data-ttl="' + ttl + '" data-en="' + esc(en) + '">' + esc(L(ru, en)) + '</button>';
  }
  function ip(a) { return '<span class="ip">' + esc(a) + '</span>'; }
  function mk(key) {
    if (key === 'silent') {
      return '<svg class="mk" viewBox="0 0 10 10" aria-hidden="true" focusable="false"><rect width="10" height="10" rx="2" class="mk-bg"/><rect width="10" height="10" rx="2" fill="url(#trHatch)"/></svg>';
    }
    return '<i class="mk mk-' + key + '" aria-hidden="true"></i>';
  }
  function li(key, html) { return '<li>' + (key ? mk(key) : '') + html + '</li>'; }
  function listHL(hops) {
    var out = '';
    hops.forEach(function (h, i) {
      if (i > 0) out += i === hops.length - 1 ? S(' и ', ' and ') : S(', ', ', ');
      out += HL(h.ttl, String(h.ttl), String(h.ttl));
    });
    return out;
  }

  function conclusions(m, note) {
    var s = m.lead, w = winP(m.win), rp = roundsP(m.rounds), out = '<section class="card tr-concl" aria-label="' + esc(L('Выводы', 'Findings')) + '">';
    if (!s || !m.rounds) {
      out += '<p class="tr-lead"><strong>' + (m.range ? S('За выделенный период раундов нет.', 'No rounds in the selected period.') : S('Данных пока нет.', 'No data yet.')) + '</strong></p>';
      return out + note + '</section>';
    }
    // Over the window or over the period picked on the hop timeline.
    var pp = m.range ? periodP(m.range) : null;
    var per = pp ? { ru: 'за выделенный период ' + pp.ru, en: 'over the selected period ' + pp.en } : { ru: 'за ' + w.ru, en: 'over ' + w.en };
    var dead = !fin(s.avg) && s.lossPct > 0;
    var avg = fin(s.avg) ? ' ' + S('Средняя задержка ' + msr(s.avg).ru + '.', 'Average latency ' + msr(s.avg).en + '.', 'rest') : '';
    if (dead) {
      out += '<p class="tr-lead">' + mk('loss') + '<strong>' +
        S('Цель не ответила ни на одну пробу ' + per.ru + ' (' + rp.ru + ').', 'The destination did not answer any probe ' + per.en + ' (' + rp.en + ').') + '</strong></p>';
    } else if (s.lossPct > 0) {
      var lp = pr(s.lossPct);
      out += '<p class="tr-lead">' + mk('loss') + '<strong>' +
        S('Потери до цели ' + lp.ru + ' ' + per.ru + ' (' + rp.ru + ').', 'Loss to the destination: ' + lp.en + ' ' + per.en + ' (' + rp.en + ').') +
        '</strong>' + avg + '</p>';
    } else {
      out += '<p class="tr-lead">' + mk('ok') + '<strong>' +
        S('Потерь до цели нет ' + per.ru + ' (' + rp.ru + ').', 'No loss to the destination ' + per.en + ' (' + rp.en + ').') +
        '</strong>' + avg + '</p>';
    }
    var items = '', dest = m.dest;
    if (dead) {
      var last = null;
      m.hops.forEach(function (h) { if (!h.dest && h.s && fin(h.s.avg)) last = h; });
      items += last
        ? li('noreply', S('Последним ответил ', 'The last hop to answer is ') + HL(last.ttl, 'хоп ' + last.ttl, 'hop ' + last.ttl) +
          (last.addr ? S(' (', ' (') + ip(last.addr) + S('), хопы после него не отвечают.', '); the hops after it do not answer.')
                     : S(', хопы после него не отвечают.', '; the hops after it do not answer.')))
        : li('noreply', S('Не отвечает ни один хоп.', 'No hop answers.'));
    } else {
      var o = m.hops.filter(function (h) { return h.origin; })[0];
      if (o && o !== dest) {
        items += li('loss', S('Потери начинаются на ', 'The loss starts at ') + HL(o.ttl, 'хопе ' + o.ttl, 'hop ' + o.ttl) +
          (o.addr ? S(' (', ' (') + ip(o.addr) + S(') и продолжаются до цели.', ') and carries on to the destination.')
                  : S(' и продолжаются до цели.', ' and carries on to the destination.')));
      } else if (o && o === dest) {
        items += li('loss', S('Потери появляются только на цели, хопы до нее отвечают без потерь.', 'Loss shows up only at the destination; the hops before it answer without loss.'));
      }
    }
    var pn = probeP(m.probe);
    var sil = m.hops.filter(function (h) { return h.status === 'silent'; });
    if (sil.length === 1) {
      items += li('silent', S('Хоп ', 'Hop ') + listHL(sil) +
        S(' не отвечает на ' + pn.ru + ', а хопы после него отвечают.', ' does not answer ' + pn.en + ', while the hops after it do.'));
    } else if (sil.length > 1) {
      items += li('silent', S('Хопы ', 'Hops ') + listHL(sil) +
        S(' не отвечают на ' + pn.ru + ', а хопы после них отвечают.', ' do not answer ' + pn.en + ', while the hops after them do.'));
    }
    m.hops.filter(function (h) { return h.status === 'partial'; }).forEach(function (h) {
      var nm = nameP(h.name), name = nm ? S(' (' + nm.ru + ')', ' (' + nm.en + ')') : '';
      if (h.s) {
        // The same share as in grey in the Loss column of the hop table and the summary: «61 % без ответа».
        var nr = pr(h.s.noReplyPct), idx = m.hops.indexOf(h);
        var next = m.hops.slice(idx + 1).filter(function (x) { return x.status === 'ok' && x.s && x.s.lossPct === 0 && !x.s.noReplyPct; })[0];
        items += li('partial', HL(h.ttl, 'Хоп ' + h.ttl, 'Hop ' + h.ttl) + name +
          S(' не ответил на ' + nr.ru + ' проб', ' did not answer ' + nr.en + ' of probes') +
          (next ? S(', а ', ', while ') + HL(next.ttl, 'хоп ' + next.ttl, 'hop ' + next.ttl) + S(' после него ответил на все.', ' after it answered all of them.') : S('.', '.')));
      } else {
        items += li('partial', HL(h.ttl, 'Хоп ' + h.ttl, 'Hop ' + h.ttl) + name +
          S(' отвечает не на все пробы ' + pn.ru + '.', ' answers only some ' + pn.en + ' probes.'));
      }
    });
    if (items) out += '<ul>' + items + '</ul>';
    return out + note + '</section>';
  }

  // ---------- error texts ----------
  // One source: HE.errorInfo of the history part ({ status, text } in the interface language). Without it, a short
  // text of our own by the code before the colon («probe_unavailable: <detail>»).
  var ERR = {
    invalid_host: ['Неверное имя или адрес.', 'Invalid name or address.'],
    resolve_failed: ['Не удалось найти адрес по имени.', 'Could not resolve the name.'],
    no_ipv4: ['У этого имени нет адреса IPv4.', 'This name has no IPv4 address.'],
    probe_unavailable: ['Этот тип пробы сейчас недоступен.', 'This probe type is not available now.'],
    invalid_interval: ['Такой интервал не поддерживается.', 'This interval is not supported.'],
    no_such_target: ['Этой цели уже нет.', 'This target is gone.'],
    no_such_hop: ['Этого хопа уже нет в трассе.', 'This hop is no longer in the trace.'],
    not_implemented: ['Это пока не работает в этой сборке.', 'This does not work in this build yet.'],
    core_unavailable: ['Ядро программы не отвечает, запрос повторяется.', 'The app core does not answer; retrying.'],
    db: ['Не удалось прочитать или записать историю.', 'Could not read or write the history.'],
    io: ['Не удалось записать файлы.', 'Could not write the files.'],
  };
  function errText(code, ctx) {
    var raw = String(code == null ? '' : code.message || code), e = null;
    try { e = window.HE && HE.errorInfo ? HE.errorInfo(raw, ctx) : null; } catch (x) { e = null; }
    if (e && typeof e.text === 'string' && e.text) return e.text;
    var own = ERR[raw.split(':')[0].trim()];
    return own ? L(own[0], own[1]) : L('Не получилось: ' + raw, 'Failed: ' + raw);
  }
  function ctxOf(t) { return { host: t.host, probe: t.probe, interval: t.interval }; }
  // The neutral line under the findings: no snapshot from the core, the last failed action, or the error of a row
  // whose trace is still on screen (without a trace the calm card says it).
  function noteHtml(st, m) {
    var t = m.kind === 'down' ? '' : st.down ? errText('core_unavailable') : st.note ||
      (m.kind === 'trace' && m.target.error ? errText(m.target.error, ctxOf(m.target)) : '');
    return t ? '<p class="tr-note" role="status" data-k="note">' + esc(t) + '</p>' : '';
  }

  // Calm card instead of the trace: no targets, an unknown target, no rounds yet, or no data from the core.
  function emptyCard(m, note) {
    var lead, rest, t = m.target;
    if (m.kind === 'down') {
      return '<section class="card tr-concl" role="status"><p class="tr-lead"><strong>' + esc(errText('core_unavailable')) + '</strong></p></section>';
    }
    if (m.kind === 'none') {
      lead = ['Целей пока нет.', 'No targets yet.']; rest = ['Добавьте цель на экране «Все цели».', 'Add one on the All targets screen.'];
    } else if (m.kind === 'missing') {
      lead = ['Такой цели нет.', 'There is no such target.']; rest = ['Возможно, ее убрали. Остальные цели на экране «Все цели».', 'It may have been removed. The other targets are on the All targets screen.'];
    } else if (t.error) {
      var et = errText(t.error, ctxOf(t));
      lead = ['Цель не опрашивается.', 'The target is not probed.']; rest = [et, et];
    } else if (t.paused) {
      lead = ['Данных пока нет: цель на паузе.', 'No data yet: the target is paused.']; rest = ['Нажмите «Продолжить», чтобы начать опрос.', 'Press Resume to start probing.'];
    } else {
      lead = ['Данных пока нет: идет первый раунд.', 'No data yet: the first round is running.']; rest = ['Выводы появятся после нескольких раундов.', 'Findings appear after a few rounds.'];
    }
    return '<section class="card tr-concl" role="status"><p class="tr-lead"><strong>' + S(lead[0], lead[1]) + '</strong> ' + S(rest[0], rest[1], 'rest') + '</p>' + note + '</section>';
  }

  // ---------- hop table ----------
  var NUMS = [['last', 'c-last'], ['avg', 'c-avg'], ['min', 'c-min'], ['max', 'c-max'], ['jitter', 'c-jit']];

  function head(m, mx) {
    var c = '<div class="tr-hd" role="row">';
    c += '<div class="g lat" style="grid-column:6 / 11">' + S('задержка, мс', 'latency, ms') + '</div>';
    c += '<div class="g scale" style="grid-column:11"><span class="num">0</span><span class="num">' + P(msi(m.scale)) + '</span></div>';
    c += '<div class="c-ttl r" role="columnheader">' + S('Хоп', 'Hop') + '</div>';
    c += '<div class="c-addr" role="columnheader">' + S('Адрес', 'Address') + '</div>';
    c += '<div class="c-name" role="columnheader">' + S('Имя', 'Name') + '</div>';
    c += '<div class="c-loss" role="columnheader"><span class="lh">' + S('Потери', 'Loss') + '</span></div>';
    c += '<div class="c-sent r" role="columnheader" title="' + esc(m.range ? L('Отправлено проб за выделенный период', 'Probes sent in the selected period') : L('Отправлено проб за окно', 'Probes sent in the window')) + '">' + S('Отпр.', 'Sent') + '</div>';
    c += '<div class="c-last r" role="columnheader" title="' + esc(L('Последний ответ', 'Last reply')) + '">' + S('Посл.', 'Last') + '</div>';
    c += '<div class="c-avg r" role="columnheader" title="' + esc(L('Среднее', 'Average')) + '">' + S('Сред.', 'Avg') + '</div>';
    c += '<div class="c-min r" role="columnheader">' + S('Мин.', 'Min') + '</div>';
    c += '<div class="c-max r" role="columnheader">' + S('Макс.', 'Max') + '</div>';
    c += '<div class="c-jit r" role="columnheader">' + S('Джиттер', 'Jitter') + '</div>';
    var sc = msi(m.scale);
    c += '<div class="c-band" role="columnheader" title="' + esc(L('Полоса от минимума до максимума, риска: среднее; шкала 0-' + sc.ru + ' над колонкой', 'Bar from minimum to maximum, tick: average; scale 0-' + sc.en + ' above the column')) + '">' + S('Полоса задержки', 'Latency band') + '</div>';
    if (mx) c += '<div class="c-probes" role="columnheader">' + m.probes.map(function (k) { return '<span>' + P(probeP(k)) + '</span>'; }).join('') + '</div>';
    c += '<div class="c-status" role="columnheader">' + S('Статус', 'Status') + '</div>';
    c += '<div class="c-pin" role="columnheader"></div>';
    return c + '</div>';
  }

  function band(h, scale) {
    var s = h.s;
    // A node that never answered (in the window or the picked period) gets the track only.
    if (s && !fin(s.min)) return '<div class="band"><div class="trk"></div></div>';
    if (!s) {
      if (h.status === 'silent') {
        return '<div class="band"><svg aria-hidden="true" focusable="false"><rect width="100%" height="8" rx="4" class="mk-bg"/><rect width="100%" height="8" rx="4" fill="url(#trHatchL)"/></svg></div>';
      }
      return '<div class="band"><div class="trk"></div></div>';
    }
    var X = function (v) { return Math.max(0, Math.min(100, (v / scale) * 100)); }, warn = warnMs(), bad = badMs(), z = '';
    if (scale > warn) z += '<i class="z-warn" style="left:' + X(warn).toFixed(2) + '%;width:' + (X(bad) - X(warn)).toFixed(2) + '%"></i>';
    if (scale > bad) z += '<i class="z-bad" style="left:' + X(bad).toFixed(2) + '%;right:0"></i>';
    return '<div class="band"><div class="trk">' + z +
      '<i class="rng" style="left:' + X(s.min).toFixed(2) + '%;width:' + (X(s.max) - X(s.min)).toFixed(2) + '%"></i></div>' +
      (fin(s.avg) ? '<i class="avg" style="left:' + X(s.avg).toFixed(2) + '%"></i>' : '') + '</div>';
  }

  function statusEl(h, m) {
    var p = stP(h.status, m.probe, h.dest);
    var lab = h.origin ? { ru: WT.statuses.origin.ru, en: WT.statuses.origin.en } : p;
    var tip = L(p.hru, p.hen);
    if (h.status === 'partial' && h.s) {
      var a = pr(h.s.noReplyPct), lp = pr(h.s.lossPct);
      tip += ' ' + (m.range
        ? L('За выделенный период без ответа ' + a.ru + ' проб, потерь ' + lp.ru + '.', 'No reply to ' + a.en + ' of probes in the selected period, ' + lp.en + ' loss.')
        : L('За окно без ответа ' + a.ru + ' проб, потерь ' + lp.ru + '.', 'No reply to ' + a.en + ' of probes in the window, ' + lp.en + ' loss.'));
    }
    return '<span class="st' + (h.origin ? ' origin' : '') + '" tabindex="0" data-tip-t="' + esc(L(lab.ru, lab.en)) + '" data-tip="' + esc(tip) + '">' +
      mk(h.status) + P(lab, 'tx') + '</span>';
  }

  function pinTip(on) {
    return on
      ? ' data-tip-t="' + esc(L('На главном экране', 'On the main screen')) + '" data-tip="' + esc(L('Хоп показан отдельным таймлайном. Нажмите, чтобы убрать.', 'The hop has its own timeline there. Click to remove it.')) + '"'
      : ' data-tip-t="' + esc(L('Вынести на главный экран', 'Pin to main screen')) + '" data-tip="' + esc(L('Хоп появится на главном экране отдельным таймлайном и строкой сводки.', 'The hop gets its own timeline and summary row on the main screen.')) + '"';
  }
  function pinBtn(h) {
    if (!h.addr || h.dest) return '';
    return '<button class="pin" data-pin="' + h.ttl + '" aria-pressed="' + h.pinned + '" aria-label="' +
      esc(h.pinned ? L('На главном экране', 'On the main screen') : L('Вынести на главный экран', 'Pin to main screen')) + '"' + pinTip(h.pinned) + '>' +
      (h.pinned ? ICON.pinned : ICON.pin) + '</button>';
  }

  // Column «Пробы» (2B): a mark per probe type of the target; the shown type follows the picked period.
  function probeMarks(h, m) {
    return m.probes.map(function (k) {
      var st = k === m.probe ? h.status : h.by[k] || null;
      if (!st) return '<span><span class="pm"><i class="mk mk-off"></i></span></span>';
      var p = stP(st, k, h.dest);
      return '<span><span class="pm" tabindex="0" data-tip-t="' + esc(probeLabel(k) + ': ' + L(p.ru, p.en)) + '" data-tip="' + esc(L(p.hru, p.hen)) + '">' + mk(st) + '</span></span>';
    }).join('');
  }

  function row(h, m, mx) {
    var s = h.s, sel = m.selected === h.ttl, c = '';
    c += '<div class="c-ttl r num">' + h.ttl + '</div>';
    c += '<div class="c-addr mono">' + (h.addr ? esc(h.addr) : '<span class="unk" title="' + esc(L('Адрес неизвестен: узел не ответил', 'Address unknown: the node did not answer')) + '">*</span>') + '</div>';
    var nm = nameP(h.name);
    c += '<div class="c-name"' + (nm ? ' title="' + esc(L(nm.ru, nm.en)) + '"' : '') + '>' + (nm ? P(nm) : '') + '</div>';
    if (s) {
      // One rule with the summary: real loss in «Потери»; a node that answers only some probes gets the no-reply share in grey.
      var nr = h.status === 'partial' && s.noReplyPct > 0 ? pr(s.noReplyPct) : null;
      c += '<div class="c-loss num' + (s.lossPct > 0 ? ' has' : '') + '"><span class="lv">' + (s.lossPct > 0 ? '<i class="mk mk-loss" aria-hidden="true"></i>' : '') + P(pr(s.lossPct)) + '</span>' +
        (nr ? S(nr.ru + ' без ответа', nr.en + ' no reply', 'nr') : '') + '</div>';
    } else {
      c += '<div class="c-loss num"></div>';
    }
    c += '<div class="c-sent r num">' + (s && s.sent != null ? s.sent : m.rounds) + '</div>';
    // No replies at all: no latency numbers (the mock formula gives jitter 0 there, the core gives null).
    var replied = s && fin(s.avg);
    NUMS.forEach(function (k) {
      var v = replied ? s[k[0]] : null;
      c += '<div class="' + k[1] + ' r num v">' + (fin(v) ? S(nf(v, 'ru'), nf(v, 'en')) : '') + '</div>';
    });
    c += '<div class="c-band">' + band(h, m.scale) + '</div>';
    if (mx) c += '<div class="c-probes">' + probeMarks(h, m) + '</div>';
    c += '<div class="c-status">' + statusEl(h, m) + '</div>';
    c += '<div class="c-pin">' + pinBtn(h) + '</div>';
    return '<div class="tr-row" role="row" data-ttl="' + h.ttl + '" aria-selected="' + sel + '" tabindex="' + (sel ? 0 : -1) + '">' + c + '</div>';
  }

  function table(m) {
    // Several probe types: the column «Пробы» compares them per hop (2B). One type: its status column says it all.
    var mx = m.probes.length > 1;
    var style = mx ? ' style="--pcol:' + (m.probes.length * 52 + (m.probes.length - 1) * 4) + 'px"' : '';
    // has-nr: a node with partial replies, the Loss column needs room for the no-reply share.
    var nr = m.hops.some(function (r) { return r.status === 'partial' && r.s && r.s.noReplyPct > 0; });
    var h = '<section class="card tr-tbl' + (mx ? ' mx' : '') + (nr ? ' has-nr' : '') + '"' + style + ' role="table" aria-label="' + esc(L('Хопы до цели', 'Hops to the destination')) + '">';
    h += head(m, mx);
    m.hops.forEach(function (r) { h += row(r, m, mx); });
    return h + '</section>';
  }

  // ---------- timeline of the selected hop ----------
  // A node that answers only some probes: its missing replies are skipped but the line does not break
  // (at 64 % no reply only lone dots would stay). Loss and empty buckets break it.
  function lineThrough(v, max, lossIdx, partIdx, w, h) {
    var loss = setOf(lossIdx), part = setOf(partIdx), d = '', pen = false;
    for (var i = 0; i < WT.N; i++) {
      if (loss[i]) { pen = false; continue; }
      if (v[i] == null) { if (!part[i]) pen = false; continue; }
      var x = (i / Math.max(1, WT.N - 1)) * w, y = h - (Math.min(v[i], max) / max) * h;
      d += (pen ? 'L' : 'M') + x.toFixed(1) + ' ' + y.toFixed(1) + ' ';
      pen = true;
    }
    return d || 'M0 0';
  }
  // Loss columns: adjacent marks merge into one column a step wide, otherwise a 1 px gap shows
  // between neighbours (width w/N at a step of w/(N-1)).
  function barsRuns(idx, top, height, w) {
    var step = w / Math.max(1, WT.N - 1), s = (idx || []).slice().sort(function (a, b) { return a - b; }), d = '';
    for (var k = 0; k < s.length; k++) {
      var i = s[k], j = i;
      while (k + 1 < s.length && s[k + 1] === j + 1) j = s[++k];
      var a = Math.max(0, i * step - step / 2), b = Math.min(w, j * step + step / 2);
      d += 'M' + a.toFixed(1) + ' ' + top + 'H' + b.toFixed(1) + 'V' + (top + height) + 'H' + a.toFixed(1) + 'Z';
    }
    return d || 'M0 0';
  }
  // "Answered only some probes": short separate marks narrower than the step, not a solid strip.
  function ticks(idx, top, height, w) {
    var step = w / Math.max(1, WT.N - 1), half = step * 0.3, d = '';
    (idx || []).forEach(function (i) {
      var a = Math.max(0, i * step - half), b = Math.min(w, i * step + half);
      d += 'M' + a.toFixed(1) + ' ' + top + 'H' + b.toFixed(1) + 'V' + (top + height) + 'H' + a.toFixed(1) + 'Z';
    });
    return d || 'M0 0';
  }
  function chartSvg(src, H, withGrid, sel) {
    var max = topOf(src), v = WT.series(src.series), gaps = (src.loss || []).concat(src.partial || []), warn = warnMs(), bad = badMs();
    var out = '<svg class="tl-svg" viewBox="0 0 1000 ' + H + '" preserveAspectRatio="none" aria-hidden="true" focusable="false">' + selPaths(sel, H);
    if (max > warn) {
      out += '<path class="p-warn" d="' + WT.path.zone(warn, bad, max, 1000, H) + '"/>';
      if (max > bad) out += '<path class="p-bad" d="' + WT.path.zone(bad, max, max, 1000, H) + '"/>';
    }
    if (withGrid) out += '<path class="p-grid" d="M0 ' + H / 2 + 'H1000"/>';
    out += '<path class="p-line" d="' + (src.partial && src.partial.length
      ? lineThrough(v, max, src.loss, src.partial, 1000, H)
      : WT.path.line(v, max, gaps, 1000, H, 0)) + '"/>';
    if (src.loss && src.loss.length) out += '<path class="p-loss" d="' + barsRuns(src.loss, 0, H, 1000) + '"/>';
    if (src.partial && src.partial.length) out += '<path class="p-part" d="' + ticks(src.partial, H - 8, 8, 1000) + '"/>';
    return out + selEdge(sel, H) + '</svg>';
  }
  // Jitter between neighbouring replies; for a node with partial replies the missing ones do not break the line.
  function jitterThrough(v, lossIdx, partIdx, w, h) {
    var loss = setOf(lossIdx), part = setOf(partIdx), pts = [], prev = null;
    for (var i = 0; i < WT.N; i++) {
      if (loss[i] || (v[i] == null && !part[i])) { prev = null; pts.push(null); continue; }
      if (v[i] == null) continue;
      if (prev !== null) pts.push([i, Math.abs(v[i] - v[prev])]);
      prev = i;
    }
    var known = pts.filter(Boolean).map(function (p) { return p[1]; });
    if (!known.length) return 'M0 0';
    var top = Math.max.apply(null, [0.001].concat(known)), d = '', pen = false;
    pts.forEach(function (p) {
      if (!p) { pen = false; return; }
      d += (pen ? 'L' : 'M') + ((p[0] / Math.max(1, WT.N - 1)) * w).toFixed(1) + ' ' + (h - 1 - (p[1] / top) * (h - 3)).toFixed(1) + ' ';
      pen = true;
    });
    return d || 'M0 0';
  }
  function jitterSvg(src, sel) {
    var v = WT.series(src.series), gaps = (src.loss || []).concat(src.partial || []);
    var d = src.partial && src.partial.length ? jitterThrough(v, src.loss, src.partial, 1000, 16) : WT.path.jitter(v, gaps, 1000, 16);
    return '<svg class="tl-svg" viewBox="0 0 1000 16" preserveAspectRatio="none" aria-hidden="true" focusable="false">' + selPaths(sel, 16) + '<path class="p-jit" d="' + d + '"/>' + selEdge(sel, 16) + '</svg>';
  }
  function hatchSvg(H, sel) {
    return '<svg class="tl-svg" viewBox="0 0 1000 ' + H + '" preserveAspectRatio="none" aria-hidden="true" focusable="false">' + selPaths(sel, H) +
      '<path class="p-hatch" d="' + WT.path.hatch(1000, H, 16) + '"/>' + selEdge(sel, H) + '</svg>';
  }
  // Bar "period selected" in the hop timeline card: what is recalculated and how to reset it.
  function selBar(m) {
    var r = m.range, d = durP(r[1] - r[0]), t = periodP(r);
    return '<div class="tr-selbar" role="status" data-k="selbar">' + ICON.range + S('Выделен период', 'Selected period', 'sb-t') + P(t, 'num sb-r') +
      S('(' + d.ru + ')', '(' + d.en + ')', 'sb-d') + S('Таблица хопов и выводы посчитаны за этот период', 'The hop table and findings are for this period', 'sb-n') +
      '<button type="button" class="btn tr-reset" data-act="reset">' + S('Сбросить', 'Reset') + S('Esc', 'Esc', 'sb-esc') + '</button></div>';
  }
  var OV = '<div class="tr-ov" aria-hidden="true"></div>';
  function pinText(h) {
    if (!h.addr || h.dest) return '';
    return h.pinned
      ? '<button class="btn btn-ghost" data-pin="' + h.ttl + '" aria-pressed="true"' + pinTip(true) + '>' + ICON.pinned + S('На главном экране', 'On main screen') + '</button>'
      : '<button class="btn" data-pin="' + h.ttl + '" aria-pressed="false"' + pinTip(false) + '>' + ICON.pin + S('Вынести на главный экран', 'Pin to main screen') + '</button>';
  }

  function timeline(h, m) {
    var w = winP(m.win), rg = rangeP(m.win), t = ticksP(m.win), nm = nameP(h.name);
    var out = '<section class="card tr-tl" aria-label="' + esc(L('Таймлайн хопа ' + h.ttl, 'Hop ' + h.ttl + ' timeline')) + '">';
    out += '<div class="tl-head"><h2>' + S('Хоп ' + h.ttl, 'Hop ' + h.ttl) + '</h2>';
    if (h.dest) out += '<span class="chip">' + S('цель', 'destination') + '</span>';
    out += h.addr ? '<span class="addr mono">' + esc(h.addr) + '</span>' : '<span class="nm">' + S('адрес неизвестен', 'address unknown') + '</span>';
    if (nm) out += '<span class="nm">' + P(nm) + '</span>';
    out += statusEl(h, m);
    // How to get the table for a period; while one is picked, the "Selected period" bar stands instead.
    out += m.range ? '<span class="sp"></span>' : S('Выделите участок мышью, чтобы посчитать таблицу хопов за этот период', 'Drag across the chart to get the hop table for that period', 'tr-tl-hint');
    out += '<span class="when">' + S(w.ru + ', ' + rg.ru, w.en + ', ' + rg.en) + '</span>';
    out += pinText(h) + '</div>';
    if (m.range) out += selBar(m);

    var src = h.src, max = src ? topOf(src) : 0, sel = m.idx;
    out += '<div class="tl-body" data-k="body">';
    out += '<div class="tl-gl mid"><span>' + S('джиттер', 'jitter') + '</span></div>';
    out += '<div class="tl-well tl-jit" data-plot="jit">' + (src ? jitterSvg(src, sel) : h.status === 'silent' ? hatchSvg(16, sel) : '') + OV + '</div>';
    if (src) {
      out += '<div class="tl-gl"><span style="top:1px;transform:none" class="num">' + P(msi(max)) + '</span><span style="top:50%" class="num">' + P(nip(max / 2)) + '</span><span style="bottom:0;top:auto;transform:none" class="num">0</span></div>';
      out += '<div class="tl-well tl-chart" data-plot="hop">' + chartSvg(src, 120, true, sel) + OV + '</div>';
    } else {
      // The node does not answer probes of this type: only the hatch, the status and its hint are in the head.
      // A hop that has only a status for this probe type gets a short note on the field.
      var pn = probeP(m.probe), quiet = h.status === 'silent' || h.status === 'noreply';
      out += '<div class="tl-gl"></div>';
      out += '<div class="tl-well tl-chart" data-plot="hop">' + (h.status === 'silent' ? hatchSvg(120, sel) : '<svg class="tl-svg" viewBox="0 0 1000 120" preserveAspectRatio="none" aria-hidden="true" focusable="false">' + selPaths(sel, 120) + selEdge(sel, 120) + '</svg>') +
        (quiet ? '' : '<div class="tl-note">' + P({ ru: 'Для ' + pn.ru + ' у этого хопа есть только статус, цифр нет.', en: 'Over ' + pn.en + ' this hop has only a status, no numbers.' }) + '</div>') + OV + '</div>';
    }
    var dest = m.dest;
    if (dest && dest !== h && dest.src) {
      var ds = dest.s, davg = fin(ds.avg) ? { ru: ', сред. ' + msr(ds.avg).ru, en: ', avg ' + msr(ds.avg).en } : { ru: '', en: '' };
      out += '<div></div><div class="tl-sub">' + S('Цель для сравнения', 'Destination for comparison') + '<b class="ip">' + esc(dest.addr || m.target.host) + '</b>' +
        S('потери ' + pr(ds.lossPct).ru + davg.ru, 'loss ' + pr(ds.lossPct).en + davg.en) + (sel ? S('за выделенный период', 'in the selected period') : '') + '</div>';
      out += '<div class="tl-gl mid"><span>' + S('цель', 'dest.') + '</span></div>';
      out += '<div class="tl-well tl-dest" data-plot="dest">' + chartSvg(dest.src, 44, false, sel) + OV + '</div>';
    }
    out += '<div></div><div class="tl-x">' + t.map(function (x, i) {
      return '<span style="left:' + ((i / Math.max(1, t.length - 1)) * 100).toFixed(3) + '%">' + P(x) + '</span>';
    }).join('') + '</div>';
    out += '</div></section>';
    return out;
  }

  // ---------- update in place ----------
  // Turns the children of `el` into `html` keeping the same elements: only changed attributes and text are written,
  // so focus, :hover, scroll and pointer capture survive a live update. Elements match by position, tag and
  // data-ttl / data-k; a mismatch replaces the element.
  var tpl = document.createElement('template');
  function keyOf(n) { return n.nodeType === 1 ? n.getAttribute('data-ttl') || n.getAttribute('data-k') : null; }
  function syncAttrs(a, b) {
    var i, at;
    for (i = a.attributes.length - 1; i >= 0; i--) { at = a.attributes[i].name; if (!b.hasAttribute(at)) a.removeAttribute(at); }
    for (i = 0; i < b.attributes.length; i++) { at = b.attributes[i]; if (a.getAttribute(at.name) !== at.value) a.setAttribute(at.name, at.value); }
  }
  function same(x, y) { return x.nodeType === y.nodeType && x.nodeName === y.nodeName && keyOf(x) === keyOf(y); }
  function syncKids(a, b) {
    var list = Array.prototype.slice.call(b.childNodes);
    for (var i = 0; i < list.length; i++) {
      var x = a.childNodes[i], y = list[i];
      if (x && !same(x, y) && (keyOf(x) != null || keyOf(y) != null)) {
        // One keyed element came or went (the period bar, a hop row): step over it instead of rebuilding the rest.
        if (x.nextSibling && same(x.nextSibling, y)) { a.removeChild(x); x = a.childNodes[i]; }
        else if (list[i + 1] && same(x, list[i + 1])) { a.insertBefore(y, x); continue; }
      }
      if (!x) a.appendChild(y);
      else if (!same(x, y)) a.replaceChild(y, x);
      else if (x.nodeType === 1) { syncAttrs(x, y); syncKids(x, y); }
      else if (x.nodeValue !== y.nodeValue) x.nodeValue = y.nodeValue;
    }
    while (a.childNodes.length > list.length) a.removeChild(a.lastChild);
  }
  function morph(el, html) { tpl.innerHTML = html; syncKids(el, tpl.content); }

  // A dialog over the screen (the report) takes Esc itself.
  function modalOpen() {
    var d = document.querySelector('[aria-modal="true"]');
    return !!(d && d.getClientRects().length);
  }

  // ---------- the screen with its behaviour ----------
  // o: id, win, selected (ttl), range, loading, down, note; callbacks onBack, onReport, onWindow(w), onInterval(s),
  // onPause(paused), onProbe(rowId), onPin(ttl, pinnedRowId | null), onRange([fromMs, toMs) | null), onSelect(ttl).
  function mount(el, o) {
    ensureDefs();
    o = o || {};
    var st = {
      id: o.id || null, win: o.win || '10m', selected: o.selected == null ? null : o.selected, range: o.range || null,
      dataRange: null, menu: false, loading: o.loading !== false, pausedAt: null, down: !!o.down, note: o.note || null,
    };
    var cur = { kind: 'loading' }, hov = null, drag = null, hlTtl = null, dropTimer = null, escModal = false;
    function emit(name, a, b) { if (typeof o[name] === 'function') o[name](a, b); }

    function screenHtml(m) {
      var note = noteHtml(st, m);
      if (m.kind === 'none' || m.kind === 'missing' || m.kind === 'wait' || m.kind === 'down') return header(m, st) + '<div class="tr-main">' + emptyCard(m, note) + '</div>';
      if (m.kind === 'loading') return header(m, st) + '<div class="tr-main"></div>';
      m.h = hopOf(m, m.selected) || m.dest || m.hops[m.hops.length - 1];
      return header(m, st) + '<div class="tr-main">' + conclusions(m, note) + table(m) + timeline(m.h, m) + '</div>';
    }
    function render(focusRow) {
      var t = st.id ? byId(st.id) : null;
      if (t && t.paused) { if (!st.pausedAt || st.pausedAt.id !== t.id) st.pausedAt = { id: t.id, at: pauseTime(t) }; } else st.pausedAt = null;
      var m = model(st);
      // A picked period that left the window (it slides in live mode, or a shorter window was chosen) is dropped.
      if (m.kind === 'trace' && st.range && !m.idx) {
        st.range = null; m = model(st);
        if (!dropTimer) dropTimer = setTimeout(function () { dropTimer = null; emit('onRange', null); }, 0);
      }
      if (m.kind === 'trace' && st.selected == null) st.selected = m.selected;
      cur = m;
      morph(el, '<div class="screen">' + screenHtml(m) + '</div>');
      if (focusRow) {
        var r = el.querySelector('.tr-row[data-ttl="' + m.selected + '"]');
        if (r) r.focus();
      }
      // Hover marks are not part of the markup: put them back.
      if (hlTtl != null) {
        var hr = el.querySelector('.tr-row[data-ttl="' + hlTtl + '"]');
        if (hr) hr.classList.add('is-hover');
      }
      if (drag) dragPaint(); else if (hov != null && m.kind === 'trace') hoverAt(hov); else hoverOff();
      tipRefresh();
    }
    function setRange(r) { st.range = r; render(); emit('onRange', r); }

    // ---------- hop timeline: hover hint and picking a period, as on the main screen timelines ----------
    function wellOf(e) { var w = e.target.closest ? e.target.closest('.tr-tl .tl-well[data-plot]') : null; return w && el.contains(w) ? w : null; }
    // All fields of the card share the x axis of the hop chart.
    function idxAt(x) {
      var c = el.querySelector('.tr-tl .tl-well[data-plot="hop"]'), r = c && c.getBoundingClientRect();
      if (!r || !r.width) return 0;
      return clampI(Math.round((x - r.left) / r.width * (WT.N - 1)));
    }
    function msIdx(ms) { return Math.round((ms - WT.fromMs) / WT.bucketMs); }
    // The selection is drawn in every field of the hop timeline (jitter, hop, destination).
    function paintSel(a, b) {
      el.querySelectorAll('.tr-tl .tl-well svg').forEach(function (s) {
        var H = +s.getAttribute('viewBox').split(' ')[3], p = s.querySelector('.p-sel'), g = s.querySelector('.p-sel-edge');
        if (p) p.setAttribute('d', a == null ? 'M0 0' : rangePath(a, b, H));
        if (g) g.setAttribute('d', a == null ? 'M0 0' : edgePath(a, b, H));
      });
    }
    function hoverAt(i, label) {
      if (cur.kind !== 'trace') return;
      var x = (i / Math.max(1, WT.N - 1) * 100).toFixed(2), dest = cur.dest;
      el.querySelectorAll('.tr-tl .tl-well[data-plot]').forEach(function (w) {
        var ov = w.querySelector('.tr-ov'), kind = w.getAttribute('data-plot'), h = '<i class="tr-xh" style="left:' + x + '%"></i>';
        var hp = kind === 'hop' ? cur.h : kind === 'dest' ? dest : null, src = hp && hp.src;
        var v = src ? WT.series(src.series)[i] : null;
        if (src && !label && v != null && !setOf(src.loss)[i]) {
          var top = topOf(src), H = kind === 'dest' ? 44 : 120, y = (H - Math.min(v, top) / top * H) / H * 100;
          h += '<i class="tr-dot' + (kind === 'dest' ? ' d2' : '') + '" style="left:' + x + '%;top:' + y.toFixed(2) + '%"></i>';
        }
        if (ov) ov.innerHTML = h;
      });
      var chart = el.querySelector('.tr-tl .tl-well[data-plot="hop"]');
      if (chart) plotTipShow(chart, i, label || plotTip(cur.h, cur, i));
    }
    function hoverOff() {
      el.querySelectorAll('.tr-ov').forEach(function (ov) { ov.innerHTML = ''; });
      plotTipHide();
    }
    // Drag ends are kept as bucket start times: live data may shift the window under the pointer.
    function dragPaint() {
      var a = Math.min(drag.a, drag.b), b = Math.max(drag.a, drag.b), p = periodP([a, b + WT.bucketMs]), d = durP(b + WT.bucketMs - a);
      paintSel(clampI(msIdx(a)), clampI(msIdx(b)));
      hoverAt(clampI(msIdx(drag.b)), '<span class="tt num">' + esc(L(p.ru, p.en)) + '</span>' + esc(L(d.ru, d.en)));
    }
    el.addEventListener('pointermove', function (e) {
      if (drag) { drag.b = bAt(idxAt(e.clientX)); dragPaint(); return; }
      var w = wellOf(e);
      if (!w || cur.kind !== 'trace') { if (hov != null) { hov = null; hoverOff(); } return; }
      hov = idxAt(e.clientX);
      hoverAt(hov);
    });
    el.addEventListener('pointerleave', function () { if (!drag) { hov = null; hoverOff(); } });
    el.addEventListener('pointerdown', function (e) {
      if (e.button !== 0 || cur.kind !== 'trace') return;
      var w = wellOf(e);
      if (!w) return;
      e.preventDefault();
      var t = bAt(idxAt(e.clientX));
      drag = { w: w, a: t, b: t, id: e.pointerId };
      try { w.setPointerCapture(e.pointerId); } catch (x) { /* synthetic event: the drag ends on pointerup */ }
    });
    function endSel() {
      if (!drag) return;
      var d = drag, a = Math.min(d.a, d.b), b = Math.max(d.a, d.b);
      drag = null;
      try { d.w.releasePointerCapture(d.id); } catch (x) { /* already released */ }
      if (b - a >= WT.bucketMs) { setRange([a, b + WT.bucketMs]); return; } // at least two buckets
      paintSel(cur.idx ? cur.idx[0] : null, cur.idx ? cur.idx[1] : null);
      if (hov != null) hoverAt(hov); else hoverOff();
    }
    // Esc during a drag drops it; the period picked before stays. The pointer is where the drag got to.
    function cancelSel() {
      var d = drag;
      drag = null;
      try { d.w.releasePointerCapture(d.id); } catch (x) { /* already released */ }
      paintSel(cur.idx ? cur.idx[0] : null, cur.idx ? cur.idx[1] : null);
      hov = clampI(msIdx(d.b));
      hoverAt(hov);
    }
    el.addEventListener('pointerup', endSel);
    el.addEventListener('pointercancel', endSel);
    window.addEventListener('pointerup', endSel);
    window.addEventListener('scroll', function () { if (!drag && hov != null) { hov = null; hoverOff(); } }, true);
    // The interval menu closes; the focus goes back to its button when it was in the menu (the item is gone).
    function menuFocused() {
      var a = document.activeElement;
      return !!(a && a.closest && a.closest('.tr-menu') && el.contains(a));
    }
    function closeMenu(refocus) {
      st.menu = false;
      render();
      var b = refocus && el.querySelector('[data-act="interval"]');
      if (b) b.focus();
    }
    // Esc: a drag first (window, capture: before any other Esc handler, as in the summary). Then, once the
    // others had their turn (window, bubbling), the interval menu, then the picked period. A dialog over the
    // screen takes Esc itself: it is noted before anyone handles the key, so its Esc does not reach the screen.
    window.addEventListener('keydown', function (e) {
      if (e.key !== 'Escape') return;
      escModal = modalOpen();
      if (!drag) return;
      e.preventDefault(); e.stopPropagation();
      cancelSel();
    }, true);
    window.addEventListener('keydown', function (e) {
      if (e.key !== 'Escape' || e.defaultPrevented || escModal) return;
      if (st.menu) { e.preventDefault(); closeMenu(menuFocused()); return; }
      if (st.range) { e.preventDefault(); setRange(null); }
    });
    el.addEventListener('click', function (e) {
      var t = e.target.closest('[data-pin],[data-act],[data-win],[data-probe],[data-iv],[data-ttl]');
      if (t && !el.contains(t)) t = null;
      var keepMenu = t && (t.getAttribute('data-act') === 'interval' || t.hasAttribute('data-iv'));
      if (st.menu && !keepMenu) { st.menu = false; render(); }
      if (!t) return;
      if (t.hasAttribute('data-pin')) {
        var hp = cur.kind === 'trace' ? hopOf(cur, +t.getAttribute('data-pin')) : null;
        if (hp) emit('onPin', hp.ttl, hp.pin);
      } else if (t.hasAttribute('data-act')) {
        var a = t.getAttribute('data-act');
        if (a === 'back') emit('onBack');
        else if (a === 'report') { if (!t.disabled) emit('onReport'); }
        else if (a === 'pause') emit('onPause', !cur.paused);
        else if (a === 'interval') { st.menu = !st.menu; render(); }
        else if (a === 'reset') setRange(null);
      } else if (t.hasAttribute('data-iv')) {
        closeMenu(menuFocused());
        emit('onInterval', +t.getAttribute('data-iv'));
      } else if (t.hasAttribute('data-win')) {
        var w = t.getAttribute('data-win');
        if (w !== st.win) { st.win = w; render(); emit('onWindow', w); }
      } else if (t.hasAttribute('data-probe')) {
        var k = t.getAttribute('data-probe'), pr = (cur.probeRows || []).filter(function (r) { return r.probe === k; })[0];
        if (pr && pr.id !== st.id) emit('onProbe', pr.id);
      } else if (t.hasAttribute('data-ttl')) {
        st.selected = +t.getAttribute('data-ttl');
        render(t.classList.contains('tr-row'));
        emit('onSelect', st.selected);
      }
    });
    // A click anywhere outside the screen closes the interval menu too.
    document.addEventListener('click', function (e) { if (st.menu && !el.contains(e.target)) { st.menu = false; render(); } });
    el.addEventListener('keydown', function (e) {
      var r = e.target.closest && e.target.closest('.tr-row');
      if (!r || e.target !== r) return;
      var rows = Array.prototype.slice.call(el.querySelectorAll('.tr-row')), i = rows.indexOf(r);
      var j = e.key === 'ArrowDown' ? i + 1 : e.key === 'ArrowUp' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? rows.length - 1 : -2;
      if (j === -2 || j < 0 || j >= rows.length) return;
      e.preventDefault();
      st.selected = +rows[j].getAttribute('data-ttl');
      render(true);
      emit('onSelect', st.selected);
    });
    // Hovering «хоп N» in the findings highlights its row.
    el.addEventListener('mouseover', function (e) {
      var b = e.target.closest && e.target.closest('.hl-btn');
      el.querySelectorAll('.tr-row.is-hover').forEach(function (x) { x.classList.remove('is-hover'); });
      hlTtl = b ? b.getAttribute('data-ttl') : null;
      if (b) {
        var hr = el.querySelector('.tr-row[data-ttl="' + hlTtl + '"]');
        if (hr) hr.classList.add('is-hover');
      }
    });
    render();
    // The screen reads the picked period (for the report) and feeds new data and state through update().
    return {
      st: st,
      update: function (patch) { if (patch) Object.assign(st, patch); render(); },
      setRange: setRange,
      model: function () { return cur; },
    };
  }

  // ---------- hop timeline hint: time, latency and loss of the hop and the destination ----------
  // Rounds per bucket at the target's interval: an estimate for when the core does not count the probes.
  function perBucket(m) { var iv = m.target.interval > 0 ? m.target.interval : 2.5; return Math.max(1, Math.round(WT.bucketMs / 1000 / iv)); }
  // Probes sent and probes without a reply in bucket i, when the core counts them (sentN, lostN).
  function countAt(src, i) {
    var n = src.sentN ? src.sentN[i] : null, k = src.lostN ? src.lostN[i] : null;
    return fin(n) && n > 0 && fin(k) ? { n: n, k: Math.max(0, Math.min(k, n)) } : null;
  }
  function cellAt(hp, probe, i, k) {
    var src = hp.src;
    if (!src) { var p = stP(hp.status, probe); return '<span class="tp-o">' + esc(L(p.ru, p.en)) + '</span>'; }
    var lost = !!setOf(src.loss)[i], part = !lost && !!setOf(src.partial)[i], v = WT.series(src.series)[i], has = v != null;
    var val = has ? msr(v) : lost || part ? { ru: 'нет ответа', en: 'no reply' } : { ru: 'нет данных', en: 'no data' };
    // A node that answers only some probes: these are replies, not loss («Ответы 1 из 2»), as in the stack hint.
    var n, c = countAt(src, i);
    if (c && part) n = mk('partial') + esc(L((c.n - c.k) + ' из ' + c.n, (c.n - c.k) + ' of ' + c.n));
    else if (c) n = (c.k ? mk('loss') : '') + esc(L(c.k + ' из ' + c.n, c.k + ' of ' + c.n));
    else if (lost) n = mk('loss') + esc(has ? (k === 2 ? L('1 из 2', '1 of 2') : L('есть', 'some')) : L(k + ' из ' + k, k + ' of ' + k));
    else if (part) n = mk('partial') + esc(has ? (k === 2 ? L('1 из 2', '1 of 2') : L('не все', 'not all')) : L('0 из ' + k, '0 of ' + k));
    else n = has ? esc(L('0 из ' + k, '0 of ' + k)) : '';
    return '<span class="tp-v">' + esc(L(val.ru, val.en)) + '</span>' +
      '<span class="tp-k">' + esc(part ? L('ответы', 'replies') : L('потери', 'loss')) + '</span>' +
      '<span class="tp-v">' + n + '</span>';
  }
  function plotTip(h, m, i) {
    var dest = m.dest, k = perBucket(m), t = tAt(i), out = '<span class="tt num">' + esc(L(t.ru, t.en)) + '</span><div class="tr-ptip-g">';
    out += '<span class="tp-l">' + esc(L('Хоп ', 'Hop ') + h.ttl) + '</span>' + cellAt(h, m.probe, i, k);
    if (dest && dest !== h && dest.src) out += '<span class="tp-l">' + esc(L('Цель', 'Destination')) + '</span>' + cellAt(dest, m.probe, i, k);
    return out + '</div>';
  }
  var ptip = null;
  function plotTipShow(chart, i, html) {
    if (!ptip) {
      ptip = document.createElement('div');
      ptip.className = 'tr-tip tr-ptip';
      ptip.setAttribute('role', 'tooltip');
      document.body.appendChild(ptip);
    }
    ptip.innerHTML = html;
    ptip.hidden = false;
    var r = chart.getBoundingClientRect(), x = r.left + r.width * i / Math.max(1, WT.N - 1), w = ptip.offsetWidth;
    ptip.style.left = Math.round(x + 14 + w > window.innerWidth - 8 ? x - 14 - w : x + 14) + 'px';
    ptip.style.top = Math.round(Math.max(8, r.top + 6)) + 'px';
  }
  function plotTipHide() { if (ptip) ptip.hidden = true; }

  // ---------- hints ----------
  var tipEl = null, tipFor = null, tipByFocus = false;
  function tipBox() {
    if (!tipEl) {
      tipEl = document.createElement('div');
      tipEl.className = 'tr-tip';
      tipEl.setAttribute('role', 'tooltip');
      tipEl.hidden = true;
      document.body.appendChild(tipEl);
    }
    return tipEl;
  }
  function tipHTML(t) {
    var text = t.getAttribute('data-tip') || '';
    return '<span class="tt">' + esc(t.getAttribute('data-tip-t')) + '</span>' + esc(text);
  }
  function tipShow(t, byFocus) {
    if (byFocus !== undefined) tipByFocus = byFocus;
    var b = tipBox();
    b.innerHTML = tipHTML(t);
    b.hidden = false;
    var r = t.getBoundingClientRect(), w = b.offsetWidth, h = b.offsetHeight;
    var x = Math.min(Math.max(8, r.left), window.innerWidth - w - 8);
    var y = r.bottom + 6;
    if (y + h > window.innerHeight - 8) y = r.top - h - 6;
    b.style.left = x + 'px';
    b.style.top = y + 'px';
    tipFor = t;
  }
  function tipHide() { if (tipEl) tipEl.hidden = true; tipFor = null; }
  // After a live update: the hint follows its element's new text, or goes when the element is gone.
  function tipRefresh() {
    if (!tipFor) return;
    if (document.body.contains(tipFor) && tipFor.hasAttribute('data-tip-t')) tipShow(tipFor); else tipHide();
  }
  document.addEventListener('mouseover', function (e) {
    var t = e.target.closest && e.target.closest('[data-tip-t]');
    if (t) { if (t !== tipFor) tipShow(t, false); else tipByFocus = false; } else if (tipFor) tipHide();
  });
  document.addEventListener('focusin', function (e) {
    var t = e.target.closest && e.target.closest('[data-tip-t]');
    if (t && t.matches(':focus-visible')) tipShow(t, true);
  });
  // The focus moving on hides a hint that the focus opened. A hint under the pointer stays: a click on another
  // control or the arrow keys in the table move the focus while the pointer is still on the element.
  document.addEventListener('focusout', function () {
    if (tipFor && (tipByFocus || !tipFor.matches(':hover'))) tipHide();
  });
  window.addEventListener('scroll', function () { tipHide(); plotTipHide(); }, true);

  window.TR = {
    EN: EN, L: L, S: S, esc: esc, nf: nf, ni: ni, pr: pr, msr: msr, ICON: ICON, stP: stP,
    model: model, mount: mount, rangeIdx: rangeIdx, ensureDefs: ensureDefs, errorText: errText,
    // Hops of the target of row rowId pinned from any of its probe rows: ttl -> pinned row id.
    pinsOf: function (rowId) { var r = rowId ? byId(rowId) : null; return r ? pinsOf(r) : {}; },
  };
})();
