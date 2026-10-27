// Whilo Trace, part "Summary": the target table of the main screen.
// Rows come from window.WT (base/wt.js, filled from the core by base/live.js): one line per target and probe
// type, the lines of one target grouped under its address, every pinned hop as a row of its own. Numbers come
// ready from the core: t.stats for the window, t.rangeStats for a period picked on a timeline [fromMs, toMs).
// The status too: t.status for the window, rangeStats.status for the period (stKey). Error texts: HE.errorInfo.
// The table is built once and then patched in place on every snapshot, so hover, the picked row, focus,
// the status tooltip and a drag in progress survive live updates.
//
//   var sum = SUM.mount(el, {
//     onTrace: function (rowId, row) {},      open the trace screen (button, double click, Enter); a hop row has hopOf and hop
//     onSelect: function (itemId) {},         the picked row changed; itemId is its first row id, null when none
//     onOrder: function (rowIds) {},          new order after a drag or Alt+arrow; without it the part calls set_order
//     onReport: function () {},               "Report for ISP" in the header of the card
//     onError: function (code, cmd, rowId) {} a pause or remove command failed (without it: the console)
//     range: function () {},                  the period [fromMs, toMs) the data on screen is for (default: Live.shown.range)
//     live: true,                             follow Live.onData; false: the screen calls sum.update() itself
//   });
//   sum.update(); sum.select(rowId); sum.selected(); sum.setOrder(rowIds); sum.order(); sum.destroy();
// Pause, resume and remove go to the core from here (set_paused, remove_target, unpin_hop); base/live.js
// asks for a fresh snapshot after them.
(function () {
  'use strict';
  var WT = window.WT, NB = ' ', EN = false;
  var HOLD_MS = 5000; // how long a local change (pause, removal, order) waits for the data to confirm it

  /* ---------- text ---------- */
  function isEN() { return document.documentElement.lang === 'en'; }
  function L(ru, en) { return EN ? en : ru; }
  function esc(s) { return String(s == null ? '' : s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  // Visible text keeps its English variant in data-en, as in the rest of the interface.
  function tx(ru, en, cls) { return '<span' + (cls ? ' class="' + cls + '"' : '') + ' data-en="' + esc(en) + '">' + esc(L(ru, en)) + '</span>'; }
  function known(n) { return typeof n === 'number' && isFinite(n); }
  // Numbers through WT.fmt (decimal comma, whole numbers from 10); "10,0" reads as "10".
  function nf(n, en) { if (!known(n)) return ''; var s = WT.fmt(n); if (s === '10,0') s = '10'; return en ? s.replace(',', '.') : s; }
  function pf(n, en) { if (!known(n)) return ''; var s = n === 0 ? '0' : nf(n, en); return en ? s + '%' : s + NB + '%'; }
  function num(n) { return nf(n, EN); }
  function pct(n) { return pf(n, EN); }
  // Interval: whole seconds without a fraction (5 с, 10 с), others through WT.fmt (2,5 с, 0,5 с).
  function sec(n) { return known(n) ? (n % 1 === 0 ? String(n) : num(n)) + (EN ? ' s' : NB + 'с') : ''; }
  function plural(n, f) { var a = n % 10, b = n % 100; return f[a === 1 && b !== 11 ? 0 : a >= 2 && a <= 4 && (b < 12 || b > 14) ? 1 : 2]; }
  function cap(s) { return s ? s.charAt(0).toUpperCase() + s.slice(1) : s; }
  var NAME_EN = { 'роутер': 'router' };
  function probeLabel(p) { var x = WT.probes[p]; return x ? x.label : String(p || '').toUpperCase(); }
  function statusOf(k) { return WT.statuses[k] || { ru: String(k || ''), en: String(k || '') }; }
  function stName(k, probe) { var st = statusOf(k); return L(st.ru, st.en).replace('{probe}', probeLabel(probe)); }

  // Status of a line. Without a picked period it is the core's (t.status) and is never counted here.
  // For a period: rangeStats.status from the core; until the core sends it, the core's rule for lines that are
  // "loss" or "ok" in the window (loss from LOSS_NOTABLE_PCT), the other statuses stay. The timelines do the same.
  var LOSS_NOTABLE_PCT = 2;
  function stKey(t, c) {
    var k = t.status, r = c && c.range ? t.rangeStats : null;
    if (r && r.status && WT.statuses[r.status]) k = r.status;
    else if (r && (k === 'loss' || k === 'ok')) k = (+r.lossPct || 0) >= LOSS_NOTABLE_PCT ? 'loss' : 'ok';
    return k;
  }

  // Error texts come from one place, HE.errorInfo of the history part (docs/ui-contract.md codes; "probe_unavailable:
  // <detail>", "io: ..." and "db: ..." carry a detail). It may answer with strings in the interface language or
  // with [ru, en] pairs; both are read here. The short texts below are only for a page without that part.
  var ERR = {
    invalid_host: ['неверный адрес', 'Invalid address'],
    resolve_failed: ['не удалось найти хост', 'Host not found'],
    no_ipv4: ['нет адреса IPv4', 'No IPv4 address'],
    probe_unavailable: ['пробы {probe} недоступны', '{probe} probes unavailable'],
  };
  function heInfo(code, ctx) {
    var H = window.HE, e = null;
    if (!H || !H.errorInfo) return null;
    try { e = H.errorInfo(code, ctx); } catch (x) { console.error(x); }
    if (!e) return null;
    var i = EN ? 1 : 0;
    function one(x) { return Array.isArray(x) ? x[i] || '' : typeof x === 'string' ? x : ''; }
    return { status: e.status, text: one(e.text) || [one(e.title), one(e.hint)].filter(Boolean).join(' ') };
  }
  // [ru, en] from a pair or from a string in the interface language; own words fill what is missing.
  function pair(x, own) {
    if (Array.isArray(x)) return [x[0] || own[0], x[1] || own[1]];
    if (typeof x === 'string' && x) return EN ? [own[0], x] : [x, own[1]];
    return own;
  }
  // A row error: the short status for the row ({ru, en}) and the full text for the tooltip (interface language).
  // A code HE.errorInfo gives no status for is one it does not know: own words with the detail read better then.
  function errOf(t) {
    var code = String(t.error), m = /^([a-z_]+)\s*(?::\s*([\s\S]+))?$/.exec(code), p = probeLabel(t.probe), base = m ? m[1] : '';
    var own = (ERR[base] || ['ошибка', 'Error']).map(function (s) { return s.replace('{probe}', p); });
    var e = heInfo(t.error, { host: t.host, probe: t.probe, interval: t.interval }), hs = pair(e && e.status, ['', '']);
    var text = e && e.text && (hs[0] || hs[1] || !ERR[base]) ? e.text : cap(L(own[0], own[1])) + (m && m[2] ? ': ' + m[2] : '') + '.';
    return { ru: hs[0] || own[0], en: hs[1] || own[1], text: text };
  }
  // A failed command in one line, for the console when the screen does not take the error itself.
  function errLine(code) { var e = heInfo(code); return (e && e.text) || String(code && code.message || code); }

  /* ---------- icons: own, one stroke, colour from currentColor ---------- */
  var I = {
    grip: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5h0M10 3.5h0M6 8h0M10 8h0M6 12.5h0M10 12.5h0"/></svg>',
    pause: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 4v8M10 4v8"/></svg>',
    play: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 3.8v8.4L12 8z"/></svg>',
    close: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/></svg>',
    trace: '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="3.5" cy="11.5" r="1.5"/><circle cx="8" cy="5" r="1.5"/><circle cx="12.5" cy="9" r="1.5"/><path d="M4.4 10.2l2.7-3.9M9.2 5.9l2.2 1.9"/></svg>',
    report: '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path d="M4 2.5h5.5L12 5v8.5H4z"/><path d="M9.5 2.5V5H12M6.2 8h3.6M6.2 10.5h3.6"/></svg>',
    hop: '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M.8 6h2.9M8.3 6h2.9"/><circle cx="6" cy="6" r="2.3"/></svg>',
    pz: '<svg class="sum-pz" viewBox="0 0 10 10" aria-hidden="true"><path d="M3 2v6M7 2v6"/></svg>',
    elbow: '<svg class="sum-elbow" viewBox="0 0 11 20" preserveAspectRatio="none" aria-hidden="true"><path d="M.75 0V20H11"/></svg>',
  };

  /* ---------- time: labels from the snapshot (local time) ---------- */
  function two(n) { return (n < 10 ? '0' : '') + n; }
  function hm(ms) { var d = new Date(ms); return two(d.getHours()) + ':' + two(d.getMinutes()); }
  function hms(ms) { return hm(ms) + ':' + two(new Date(ms).getSeconds()); }
  function midnight(ms) { var d = new Date(ms); d.setHours(0, 0, 0, 0); return d.getTime(); }
  // The day of a moment as the screen says it: today, yesterday or the date. Older days in the one date format
  // of the interface (parts/history dateOf): 25.10.2026, and 2026‑10‑25 with non-breaking hyphens.
  var NBH = '‑';
  function dayOf(ms) {
    var back = Math.round((midnight(WT.nowMs || Date.now()) - midnight(ms)) / 864e5), d = new Date(ms);
    if (back <= 0) return { ru: 'сегодня', en: 'today', today: true };
    if (back === 1) return { ru: 'вчера', en: 'yesterday' };
    return { ru: two(d.getDate()) + '.' + two(d.getMonth() + 1) + '.' + d.getFullYear(), en: d.getFullYear() + NBH + two(d.getMonth() + 1) + NBH + two(d.getDate()) };
  }
  // "сегодня 15:50-16:00"; across midnight "вчера 23:55 - сегодня 00:05".
  function spanOf(a, b, f) {
    f = f || hm;
    var A = dayOf(a), B = dayOf(b);
    if (A.ru === B.ru) return { ru: A.ru + ' ' + f(a) + '-' + f(b), en: A.en + ' ' + f(a) + '-' + f(b) };
    return { ru: A.ru + ' ' + f(a) + ' - ' + B.ru + ' ' + f(b), en: A.en + ' ' + f(a) + ' - ' + B.en + ' ' + f(b) };
  }
  // A picked period: "15:54:00-15:56:00", with the day when it is not today.
  function rangeText(r) {
    if (dayOf(r[0]).today && dayOf(r[1]).today) { var s = hms(r[0]) + '-' + hms(r[1]); return { ru: s, en: s }; }
    return spanOf(r[0], r[1], hms);
  }
  function durOf(ms) {
    var s = Math.max(0, Math.round(ms / 1000)), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60, ru = [], en = [];
    if (h) { ru.push(h + ' ч'); en.push(h + ' h'); }
    if (m) { ru.push(m + ' мин'); en.push(m + ' min'); }
    if (x || !(h || m)) { ru.push(x + ' с'); en.push(x + ' s'); }
    return { ru: ru.join(' '), en: en.join(' ') };
  }
  // The window of the data from the snapshot itself (toMs - fromMs), the requested one as a fallback.
  function winKey() {
    var span = WT.toMs - WT.fromMs, key = null, lv = window.Live;
    WT.windows.forEach(function (w) { if (Math.abs(WT.windowSeconds[w[0]] * 1000 - span) < 1000) key = w[0]; });
    return key || (lv && lv.shown && lv.shown.window) || (lv && lv.state && lv.state.window) || '10m';
  }
  function winName(key) { var w = WT.windows.filter(function (x) { return x[0] === key; })[0] || WT.windows[1]; return { ru: w[1], en: w[2] }; }

  /* ---------- mini chart for the window: latency line, loss as solid columns, "no reply" marks at the bottom ---------- */
  function X(i) { return (i / Math.max(1, WT.N - 1)) * 1000; }
  function setOf(list) { var s = {}; (list || []).forEach(function (i) { s[i] = true; }); return s; }
  function linePath(t, v, max, part) {
    var pad = 14;
    if (!part) return WT.path.line(v, max, t.loss, 1000, 120, pad);
    // A node that answers only some probes: the line goes through the replies it gave and breaks only on real loss.
    var skip = setOf(t.partial), brk = setOf(t.loss), d = '', pen = false;
    for (var i = 0; i < WT.N; i++) {
      if (brk[i]) { pen = false; continue; }
      if (skip[i]) continue;
      if (v[i] == null) { pen = false; continue; }
      var y = 120 - pad - (Math.min(v[i], max) / max) * (120 - 2 * pad);
      d += (pen ? 'L' : 'M') + X(i).toFixed(1) + ' ' + y.toFixed(1) + ' ';
      pen = true;
    }
    return d || 'M0 0';
  }
  // Columns like WT.path.bars, but neighbouring buckets as one rectangle: the bucket step is wider than
  // a column, so separate rectangles would show seams in a solid run.
  function runs(idx, top, height) {
    var s = (idx || []).slice().sort(function (a, b) { return a - b; }), d = '', bw = 1000 / WT.N;
    for (var i = 0; i < s.length;) {
      var j = i;
      while (j + 1 < s.length && s[j + 1] <= s[j] + 1) j++;
      var a = Math.max(0, X(s[i]) - bw / 2).toFixed(1), b = Math.min(1000, X(s[j]) + bw / 2).toFixed(1);
      d += 'M' + a + ' ' + top + 'H' + b + 'V' + (top + height) + 'H' + a + 'Z';
      i = j + 1;
    }
    return d || 'M0 0';
  }
  function periodPath(a, b) {
    var bw = 1000 / WT.N, x0 = Math.max(0, X(a) - bw / 2).toFixed(1), x1 = Math.min(1000, X(b) + bw / 2).toFixed(1);
    return 'M' + x0 + ' 0H' + x1 + 'V120H' + x0 + 'Z';
  }
  // Buckets of a period in ms; null when it is outside the window.
  function bucketsOf(r) {
    var b = WT.bucketMs, n = WT.N;
    if (!(b > 0) || !n) return null;
    var a = Math.max(0, Math.floor((r[0] - WT.fromMs) / b)), z = Math.min(n - 1, Math.ceil((r[1] - WT.fromMs) / b) - 1);
    return z >= a ? [a, z] : null;
  }

  /* ---------- cells ---------- */
  function isIP(s) { return /^\d{1,3}(\.\d{1,3}){3}$/.test(s) || String(s).indexOf(':') >= 0; }
  // Second label next to the address: the name for an IP, the IP for a name.
  function second(t) { var s = isIP(t.host) ? t.name : (t.ip || t.name); return s && s !== t.host ? s : null; }
  function partialOf(t) { return !!((t.partial && t.partial.length) || (t.stats && t.stats.noReplyPct > 0)); }

  function gripHtml() {
    var t = L('Перетащите или нажмите Alt+стрелку, чтобы поменять порядок', 'Drag, or press Alt+arrow, to reorder');
    return '<button class="sum-grip" type="button" title="' + esc(t) + '" aria-label="' + esc(t) + '">' + I.grip + '</button>';
  }
  // Target: the address, then the other label; a hop also says where it is ("хоп 5 на пути к 1.1.1.1").
  // On a narrow card "на пути к ..." goes first (summary.css), the full text stays in the tooltip.
  function targetHtml(t, parent) {
    var nm = second(t), parts = [], full = [];
    if (nm) {
      var ne = NAME_EN[nm] || nm;
      parts.push('<span class="sum-nm mono">' + tx(nm, ne) + '</span>');
      full.push(L(nm, ne));
    }
    if (t.hopOf) {
      var hop = t.hop != null ? { ru: 'хоп ' + t.hop, en: 'hop ' + t.hop } : { ru: 'хоп', en: 'hop' }, to = parent && parent.host;
      parts.push('<span class="sum-via">' + I.hop + tx(hop.ru, hop.en) + (to ? tx(' на пути к ' + to, ' on the path to ' + to, 'scr-via-to') : '') + '</span>');
      full.push(L(hop.ru, hop.en) + (to ? L(' на пути к ', ' on the path to ') + to : ''));
    }
    var title = t.host + (full.length ? ' ' + full.join(', ') : '');
    return '<span class="sum-tgt" title="' + esc(title) + '"><span class="sum-host mono">' + esc(t.host) + '</span>' +
      (parts.length ? '<span class="sum-sub" title="' + esc(title) + '">' + parts.join('') + '</span>' : '') + '</span>';
  }
  // Loss as in the hop table: real loss in the column, the share without a reply in grey next to it.
  function lossHtml(s, part) {
    var h = '<span class="sum-loss"><span class="v' + (s.lossPct > 0 ? ' has' : '') + '">' + esc(pct(s.lossPct)) + '</span>';
    if (part) h += '<span class="sfx">' + tx(pf(s.noReplyPct, false) + ' без ответа', pf(s.noReplyPct, true) + ' no reply') + '</span>';
    return h + '</span>';
  }
  // "Now": the pause chip while paused; at a node that answers only some probes, its last reply.
  function nowHtml(sw, part, pause) {
    if (pause) return '<span class="chip sum-pause">' + tx('пауза', 'paused') + '</span>';
    if (sw.lastLost && !part) return '<span class="sum-lost">' + tx('потеря', 'lost') + '</span>';
    return esc(num(sw.last));
  }
  // One note across avg, min and now: the short status of a row error (the full text in its tooltip),
  // no replies at all, or no data yet.
  function noteHtml(t, s, c) {
    if (t.error) {
      var e = errOf(t);
      return '<span title="' + esc(e.text) + '">' + tx(e.ru, e.en) + '</span>';
    }
    if (known(s.avg)) return '';
    var per = c.range || c.past ? { ru: 'за этот период', en: 'in this period' } : { ru: 'за ' + c.win.ru, en: 'in ' + c.win.en };
    if (s.lossPct > 0 || s.sent > 0) return tx('нет ответов ' + per.ru, 'no replies ' + per.en);
    return tx('нет данных ' + per.ru, 'no data ' + per.en);
  }

  // Cells of a row: grip, status, target, probe, interval, loss, avg, min, now, chart, actions. Always eleven,
  // so the columns a narrow card hides are the same in every row.
  var CELLS = ['', 'sum-st', '', '', 'sum-n sum-int', '', 'sum-n', 'sum-n', 'sum-n', 'sum-c-chart', 'sum-acts'];
  var PATHS = ['scr-sel', 'z-warn', 'z-bad', 'pt', 'ln', 'ls'];
  var SLOT = '<span class="sum-slot"></span>';
  function rowEl() {
    var r = document.createElement('div');
    r.className = 'sum-row';
    r.setAttribute('role', 'row');
    r.innerHTML = CELLS.map(function (c) { return '<span' + (c ? ' class="' + c + '"' : '') + ' role="cell"></span>'; }).join('');
    r.children[9].innerHTML = '<span class="sum-spark"><svg viewBox="0 0 1000 120" preserveAspectRatio="none" aria-hidden="true">' +
      PATHS.map(function (p) { return '<path class="' + p + '" d="M0 0"/>'; }).join('') + '</svg></span>';
    return r;
  }
  function itemEl(key) {
    var el = document.createElement('div');
    el.className = 'sum-item';
    el.setAttribute('role', 'rowgroup');
    el.tabIndex = 0;
    el._key = key;
    el._rows = {};
    return el;
  }
  // Patch helpers: touch the DOM only when something changed, so hover and focus stay where they are.
  function set(el, cls, h) {
    if (el.className !== cls) el.className = cls;
    if (el._h !== h) { el.innerHTML = h; el._h = h; }
  }
  function setD(p, d) { if (p.getAttribute('d') !== d) p.setAttribute('d', d); }
  function setBtn(b, icon, title) {
    if (b._i !== icon) { b.innerHTML = icon; b._i = icon; }
    if (b.getAttribute('title') !== title) { b.setAttribute('title', title); b.setAttribute('aria-label', title); }
  }
  function actBtn(a) { return '<button class="sum-ib' + (a === 'trace' ? ' scr-trace-btn' : '') + '" type="button" data-act="' + a + '"></button>'; }

  function paintSpark(sp, t, sw, part, c) {
    var p = sp._p || (sp._p = sp.querySelectorAll('path')), max = t.max > 0 ? t.max : 10, zone = WT.path.zone;
    setD(p[0], c.idx ? periodPath(c.idx[0], c.idx[1]) : 'M0 0');
    setD(p[1], max > c.warn ? zone(c.warn, c.bad, max) : 'M0 0');
    setD(p[2], max > c.bad ? zone(c.bad, 1e9, max) : 'M0 0');
    setD(p[3], part ? runs(t.partial, 86, 34) : 'M0 0');
    setD(p[4], linePath(t, sw.values || [], max, part));
    setD(p[5], runs(t.loss, 0, 120));
    var title = L('Шкала 0-' + num(max) + ' мс, ' + c.span.ru, 'Scale 0-' + num(max) + ' ms, ' + c.span.en);
    if (sp.getAttribute('title') !== title) sp.setAttribute('title', title);
  }
  // Actions: open the trace (first line), pause or resume (target lines), remove the line or unpin the hop.
  function paintActs(cell, t, first, many) {
    var hop = !!t.hopOf, shape = (first ? 'f' : 'n') + (hop ? 'h' : 't');
    if (cell._s !== shape) {
      cell.innerHTML = (first ? actBtn('trace') : SLOT) + (hop ? SLOT : actBtn('pause')) + actBtn('remove');
      cell._s = shape;
    }
    var p = probeLabel(t.probe), b = cell.querySelectorAll('button'), k = 0;
    if (first) setBtn(b[k++], I.trace, L('Открыть трассу', 'Open trace'));
    if (!hop) setBtn(b[k++], t.paused ? I.play : I.pause, t.paused ? L('Продолжить', 'Resume') : many ? L('Пауза ' + p, 'Pause ' + p) : L('Пауза', 'Pause'));
    setBtn(b[k], I.close, hop ? L('Убрать хоп с главного экрана', 'Unpin the hop from the main screen')
      : many ? L('Убрать пробу ' + p + ', история останется', 'Remove the ' + p + ' probe, history is kept')
        : L('Убрать цель, история останется', 'Remove the target, history is kept'));
  }
  // Status mark of a line: a row error is grey like "no reply"; otherwise the status of the window or the picked period.
  function markKey(t, c) { return t.error ? 'error' : stKey(t, c); }
  function markHtml(t, c) {
    var k = markKey(t, c), e = t.error ? errOf(t) : null;
    return '<span class="sum-mk ' + esc(k) + '" role="img" aria-label="' + esc(e ? L(e.ru, e.en) : stName(k, t.probe)) + '"></span>';
  }
  function paintRow(r, t, i, g, c) {
    var cell = r.children, first = i === 0, many = g.lines.length > 1, paused = !!t.paused;
    var sw = WT.stats(t), s = c.range && t.rangeStats ? t.rangeStats : sw, part = partialOf(t);
    r._t = t;
    r.classList.toggle('scr-paused', paused);
    if (cell[0]._f !== first) { cell[0].innerHTML = first ? gripHtml() : ''; cell[0]._f = first; }
    set(cell[1], 'sum-st', paused ? I.pz : markHtml(t, c));
    set(cell[2], first ? '' : 'sum-elbow-cell', first ? targetHtml(t, c.byId[t.hopOf]) : I.elbow);
    set(cell[3], '', '<span class="chip">' + esc(probeLabel(t.probe)) + '</span>');
    set(cell[4], 'sum-n sum-int', esc(sec(t.interval)));
    set(cell[5], '', lossHtml(s, part));
    var note = noteHtml(t, s, c);
    if (note) {
      set(cell[6], 'sum-none', note);
      set(cell[7], 'sum-n sum-gone', '');
      set(cell[8], 'sum-n sum-gone', '');
    } else {
      // with a picked period loss, avg and min are for the period; "now" stays the last value of the window
      set(cell[6], 'sum-n', esc(num(s.avg)));
      set(cell[7], 'sum-n', esc(num(s.min)));
      set(cell[8], 'sum-n', nowHtml(sw, part, paused && !c.past));
    }
    paintSpark(cell[9].firstChild, t, sw, part, c);
    paintActs(cell[10], t, first, many);
  }

  /* ---------- data: rows, groups, order ---------- */
  // Rows to show: the snapshot with local changes that the core has not confirmed yet.
  function rowsOf(v) {
    var now = Date.now(), over = v.over, have = {}, gone = {};
    WT.targets.forEach(function (t) { have[t.id] = true; });
    Object.keys(over).forEach(function (id) {
      if (over[id].until < now || !have[id]) delete over[id];
      else if (over[id].removed) gone[id] = true;
    });
    return WT.targets.filter(function (t) { return !gone[t.id] && !(t.hopOf && gone[t.hopOf]); }).map(function (t) {
      var o = over[t.id];
      if (!o || !('paused' in o)) return t;
      if (!!t.paused === o.paused) { delete over[t.id]; return t; }
      var c = Object.assign({}, t);
      c.paused = o.paused;
      return c;
    });
  }
  // A unit of order: all lines of one target (one per probe type) or one pinned hop.
  function groupsOf(rows) {
    var out = [], by = {};
    rows.forEach(function (t) {
      var key = t.hopOf ? 'h:' + t.id : 't:' + (t.targetId != null ? t.targetId : t.host), g = by[key];
      if (!g) { g = by[key] = { key: key, hop: !!t.hopOf, lines: [] }; out.push(g); }
      g.lines.push(t);
    });
    return out;
  }
  function flat(groups) { var out = []; groups.forEach(function (g) { g.lines.forEach(function (t) { out.push(t.id); }); }); return out; }
  // After a drop the new order shows at once and holds until a snapshot comes in that order (or HOLD_MS pass).
  function ordered(v, groups) {
    var p = v.pend;
    if (!p) return groups;
    var pos = {}, want, cur;
    flat(groups).forEach(function (id) { pos[id] = -1; });
    want = p.ids.filter(function (id) { return pos[id] === -1; });
    want.forEach(function (id, i) { pos[id] = i; });
    cur = flat(groups).filter(function (id) { return pos[id] >= 0; });
    if (Date.now() > p.until || cur.join() === want.join()) { v.pend = null; return groups; }
    var rank = groups.map(function (g, i) {
      var r = Infinity;
      g.lines.forEach(function (t) { if (pos[t.id] >= 0) r = Math.min(r, pos[t.id]); });
      return { g: g, r: r, i: i };
    });
    rank.sort(function (a, b) { return a.r - b.r || a.i - b.i; });
    return rank.map(function (x) { return x.g; });
  }
  // What the numbers are for: the picked period once the data on screen answers it, the past frame or the live window.
  function context(v) {
    var byId = {}, lv = window.Live, key = winKey();
    var r = v.o.range ? v.o.range() : lv && lv.shown ? lv.shown.range : null;
    WT.targets.forEach(function (t) { byId[t.id] = t; });
    var range = r && r.length === 2 && WT.targets.some(function (t) { return t.rangeStats; }) ? r : null, st = WT.settings || {};
    return {
      byId: byId, past: WT.live === false, range: range, idx: range ? bucketsOf(range) : null,
      win: winName(key), hour: key === '1h', span: spanOf(WT.fromMs, WT.toMs),
      // latency thresholds from the settings paint the chart background, as on the timelines
      warn: st.warnMs > 0 ? st.warnMs : 100, bad: st.badMs > 0 ? st.badMs : 200,
    };
  }

  /* ---------- the card ---------- */
  function colhead() {
    function c(ru, en, cls) { return '<span role="columnheader"' + (cls ? ' class="' + cls + '"' : '') + '>' + tx(ru, en) + '</span>'; }
    return '<div class="sum-row sum-colhead" role="row"><span role="columnheader"></span><span role="columnheader"></span>' +
      c('Цель', 'Target') + c('Проба', 'Probe') + c('Интервал', 'Interval', 'sum-r') +
      '<span role="columnheader"><span class="sum-lh">' + tx('Потери', 'Loss') + '</span></span>' +
      c('Сред., мс', 'Avg, ms', 'sum-r') + c('Мин., мс', 'Min, ms', 'sum-r') + c('Сейчас, мс', 'Now, ms', 'sum-r') +
      c('За 10 мин', 'Last 10 min', 'sum-c-chart') + '<span role="columnheader"></span></div>';
  }
  function build(v) {
    EN = v.lang = isEN();
    var r = v.root;
    r.classList.add('sum', 'layer');
    r.innerHTML = '<div class="sum-head"><h2>' + tx('Сводка', 'Summary') + '</h2><span class="sum-meta"></span>' +
      '<button type="button" class="btn btn-sm scr-report" data-act="report">' + I.report + tx('Отчет для провайдера', 'Report for ISP') + '</button></div>' +
      '<div class="sum-table" role="table" aria-label="' + esc(L('Сводка целей', 'Target summary')) + '">' + colhead() + '<div class="sum-body"></div></div>';
    v.meta = r.querySelector('.sum-meta');
    v.table = r.querySelector('.sum-table');
    v.body = r.querySelector('.sum-body');
    var ch = r.querySelector('.sum-colhead').children;
    v.colNow = ch[8];
    v.colChart = ch[9];
    v.items = {};
    v.tip = null;
  }
  // Header line: how many targets and hops, and what period the numbers cover.
  function paintHead(v, groups, c) {
    var tg = 0, hp = 0, p;
    groups.forEach(function (g) { if (g.hop) hp++; else tg++; });
    var n = {
      ru: tg + ' ' + plural(tg, ['цель', 'цели', 'целей']) + (hp ? ', ' + hp + ' ' + plural(hp, ['хоп', 'хопа', 'хопов']) : ''),
      en: tg + ' target' + (tg === 1 ? '' : 's') + (hp ? ', ' + hp + ' hop' + (hp === 1 ? '' : 's') : ''),
    };
    if (c.range) {
      var rt = rangeText(c.range), d = durOf(c.range[1] - c.range[0]);
      p = { ru: 'статистика за выделенный период ' + rt.ru + ' (' + d.ru + ')', en: 'stats for the selected period ' + rt.en + ' (' + d.en + ')' };
    } else if (c.past) p = { ru: 'статистика за ' + c.span.ru, en: 'stats for ' + c.span.en };
    else if (c.hour) p = { ru: 'статистика за последний час', en: 'stats for the last hour' };
    else p = { ru: 'статистика за последние ' + c.win.ru, en: 'stats for the last ' + c.win.en };
    set(v.meta, 'sum-meta', tx(n.ru + ' · ' + p.ru, n.en + ' · ' + p.en));
    // In the past the columns speak of the frame, not of now: "В 09:00, мс" and the chart "08:50-09:00".
    var end = hm(WT.toMs), frame = hm(WT.fromMs) + '-' + end;
    set(v.colNow, 'sum-r', c.past ? tx('В ' + end + ', мс', 'At ' + end + ', ms') : tx('Сейчас, мс', 'Now, ms'));
    set(v.colChart, 'sum-c-chart', c.past ? tx(frame, frame) : tx('За ' + c.win.ru, 'Last ' + c.win.en));
  }
  // Put children in the given order with as few moves as possible; give focus back if a move took it.
  function place(parent, els) {
    var a = document.activeElement, keep = a && parent.contains(a) ? a : null, cur = parent.firstElementChild;
    els.forEach(function (el) { if (el === cur) cur = cur.nextElementSibling; else parent.insertBefore(el, cur); });
    if (keep && keep.isConnected && document.activeElement !== keep) keep.focus({ preventScroll: true });
  }
  function paintItem(v, el, g, c) {
    el._g = g;
    if (el.getAttribute('data-id') !== g.lines[0].id) el.setAttribute('data-id', g.lines[0].id);
    el.classList.toggle('is-group', g.lines.length > 1);
    el.classList.toggle('is-hop', g.hop);
    el.classList.toggle('is-selected', v.sel === g.key);
    el.classList.toggle('is-paused', g.lines.some(function (t) { return t.paused; }));
    var seen = {}, rows = g.lines.map(function (t, i) {
      var r = el._rows[t.id] || (el._rows[t.id] = rowEl());
      seen[t.id] = true;
      paintRow(r, t, i, g, c);
      return r;
    });
    Object.keys(el._rows).forEach(function (id) { if (!seen[id]) { el._rows[id].remove(); delete el._rows[id]; } });
    place(el, rows);
  }
  function update(v) {
    if (v.dead) return;
    if (isEN() !== v.lang) { endDrag(v); build(v); } // the language changed: start the card over
    EN = v.lang;
    var rows = rowsOf(v);
    // No targets: the card is not shown, the first-run message of the screen takes its place.
    if (!rows.length) {
      endDrag(v);
      hideTip(v);
      Object.keys(v.items).forEach(function (k) { v.items[k].remove(); });
      v.items = {};
      if (v.sel) { v.sel = null; emitSel(v); }
      v.root.hidden = true;
      return;
    }
    v.root.hidden = false;
    var c = v.c = context(v), groups = ordered(v, groupsOf(rows));
    paintHead(v, groups, c);
    v.table.classList.toggle('scr-has-nr', rows.some(partialOf));
    v.root.setAttribute('data-density', rows.length >= 10 ? 'compact' : 'normal');
    var seen = {}, els = groups.map(function (g) {
      var el = v.items[g.key] || (v.items[g.key] = itemEl(g.key));
      seen[g.key] = true;
      paintItem(v, el, g, c);
      return el;
    });
    Object.keys(v.items).forEach(function (k) {
      if (seen[k]) return;
      if (v.drag && v.drag.item === v.items[k]) endDrag(v);
      v.items[k].remove();
      delete v.items[k];
      if (v.sel === k) { v.sel = null; emitSel(v); }
    });
    // while a row is being dragged the order of the others stays put; new rows wait at the end
    if (v.drag) els.forEach(function (el) { if (!el.parentNode) v.body.appendChild(el); });
    else place(v.body, els);
    paintTip(v);
  }

  /* ---------- picked row, trace, actions ---------- */
  function idOf(v, key) { return key && v.items[key] ? v.items[key]._g.lines[0].id : null; }
  function emitSel(v) { if (v.o.onSelect) v.o.onSelect(idOf(v, v.sel)); }
  function select(v, key, emit) {
    if (key && !v.items[key]) key = null;
    if (key === v.sel) return;
    v.sel = key;
    Object.keys(v.items).forEach(function (k) { v.items[k].classList.toggle('is-selected', k === key); });
    if (emit) emitSel(v);
  }
  function keyOf(v, id) {
    var key = null;
    if (id) Object.keys(v.items).forEach(function (k) { if (v.items[k]._g.lines.some(function (t) { return t.id === id; })) key = k; });
    return key;
  }
  function openTrace(v, item) { var t = item._g && item._g.lines[0]; if (t && v.o.onTrace) v.o.onTrace(t.id, t); }
  function refresh() { return window.Live && window.Live.refresh ? window.Live.refresh() : Promise.resolve(); }
  // Pause, resume or remove: the row changes at once, the command goes to the core, fresh data confirms it.
  function run(v, kind, t) {
    var cmd, args, id = t.id;
    if (kind === 'pause') {
      cmd = 'set_paused'; args = { id: id, paused: !t.paused };
      v.over[id] = { paused: !t.paused, until: Date.now() + HOLD_MS };
    } else {
      cmd = t.hopOf ? 'unpin_hop' : 'remove_target'; args = { id: id };
      v.over[id] = { removed: true, until: Date.now() + HOLD_MS };
    }
    update(v);
    window.API.call(cmd, args).then(function () { update(v); }, function (err) {
      delete v.over[id];
      update(v);
      if (v.o.onError) v.o.onError(err, cmd, id); else console.error(cmd + ' failed: ' + errLine(err), err);
    });
  }
  function domOrder(v) {
    var out = [];
    [].forEach.call(v.body.children, function (el) { if (el._g) el._g.lines.forEach(function (t) { out.push(t.id); }); });
    return out;
  }
  // New order of the rows: shown at once, given to the screen (the stack takes the same order) or saved here.
  function commit(v) {
    var ids = domOrder(v);
    v.pend = { ids: ids, until: Date.now() + HOLD_MS };
    if (v.o.onOrder) { v.o.onOrder(ids.slice()); return; }
    window.API.call('set_order', { ids: ids }).then(function () { return refresh(); }).then(function () { update(v); }, function (err) { console.error('set_order failed', err); });
  }
  // Alt+arrow on the grip: the neighbour moves instead of the row, so the grip keeps its focus.
  function nudge(v, item, dir) {
    var sib = item && (dir < 0 ? item.previousElementSibling : item.nextElementSibling);
    if (!sib || v.drag) return;
    v.body.insertBefore(sib, dir < 0 ? item.nextElementSibling : item);
    commit(v);
  }

  /* ---------- status tooltip: on the status mark and on a row focused from the keyboard ---------- */
  // The mark is only a colour; the tooltip says what the mark shows and explains it (the explanation lives only here):
  // the pause, the row error (its short status and full text) or the status of the window or the picked period.
  function tipHtml(v, t) {
    var p = probeLabel(t.probe), name, hint = '';
    if (t.paused) {
      name = L('на паузе', 'paused');
      hint = L('Пробы не отправляются, история сохраняется.', 'No probes are sent, the history is kept.');
    } else if (t.error) {
      var e = errOf(t);
      name = L(e.ru, e.en);
      hint = e.text;
    } else {
      var k = stKey(t, v.c), st = statusOf(k), s = v.c && v.c.range && t.rangeStats ? t.rangeStats : WT.stats(t);
      name = stName(k, t.probe);
      // at a target "loss" is explained by the numbers next to it; hops and other statuses get the hint,
      // except "no loss" next to loss under the threshold
      var tg = !t.hopOf && st.hint_target_ru;
      if ((t.hopOf || k !== 'loss') && !(k === 'ok' && s.lossPct > 0) && st.hint_ru) {
        hint = L(tg ? st.hint_target_ru : st.hint_ru, tg ? st.hint_target_en : st.hint_en).replace('{probe}', p);
      }
      if (partialOf(t)) {
        hint += (hint ? ' ' : '') + L('Без ответа ' + pf(s.noReplyPct, false) + ' проб, потерь ' + pf(s.lossPct, false) + '.',
          'No reply to ' + pf(s.noReplyPct, true) + ' of probes, ' + pf(s.lossPct, true) + ' loss.');
      }
    }
    // the probe type after the status, unless the status already names it ("пробы ICMP недоступны")
    return '<span class="tt">' + esc(name.indexOf(p) >= 0 ? name : name + ' · ' + p) + '</span>' + esc(hint);
  }
  function showTip(v, row) {
    if (!row || !row._t || (v.tip && v.tip.row === row)) return;
    hideTip(v);
    var el = document.createElement('div');
    el.className = 'sum-tip scr-st-tip';
    el.setAttribute('role', 'tooltip');
    v.table.appendChild(el);
    v.tip = { el: el, row: row, h: null };
    paintTip(v);
  }
  function paintTip(v) {
    var tp = v.tip;
    if (!tp) return;
    if (!v.body.contains(tp.row)) { hideTip(v); return; }
    var h = tipHtml(v, tp.row._t);
    if (tp.h !== h) { tp.el.innerHTML = h; tp.h = h; }
    var tr = v.table.getBoundingClientRect(), mr = tp.row.children[1].getBoundingClientRect(), rr = tp.row.getBoundingClientRect();
    tp.el.style.left = Math.max(0, mr.left - tr.left - 10) + 'px';
    tp.el.style.top = (rr.bottom - tr.top + 4) + 'px';
  }
  function hideTip(v) { if (v.tip) { v.tip.el.remove(); v.tip = null; } }

  /* ---------- drag to reorder (item 23 of the MVP): the same look as the stack of timelines ---------- */
  // The source fades in place, a line shows where it lands, a short token (grip, status, address, probe types)
  // follows the pointer. Esc cancels. The edges of the scrolling area scroll the screen.
  function scroller(el) {
    for (var p = el.parentElement; p; p = p.parentElement) {
      var oy = getComputedStyle(p).overflowY;
      if ((oy === 'auto' || oy === 'scroll') && p.scrollHeight > p.clientHeight) return p;
    }
    return document.scrollingElement || document.documentElement;
  }
  function tokenHtml(g, c) {
    var t = g.lines[0];
    return I.grip + '<span class="sum-mk ' + esc(markKey(t, c)) + '"></span><span class="mono">' + esc(t.host) + '</span>' +
      g.lines.map(function (x) { var p = probeLabel(x.probe); return '<span class="chip">' + tx(p, p) + '</span>'; }).join('');
  }
  function startDrag(v, e, g) {
    var item = g.closest('.sum-item');
    if (!item || !item._g) return;
    try { g.setPointerCapture(e.pointerId); } catch (x) { /* a synthetic event: the document still gets the moves */ }
    hideTip(v);
    var ins = document.createElement('div'), tok = document.createElement('div');
    ins.className = 'sum-ins'; ins.hidden = true; ins.setAttribute('aria-hidden', 'true');
    tok.className = 'sum-token'; tok.setAttribute('aria-hidden', 'true'); tok.innerHTML = tokenHtml(item._g, v.c);
    document.body.appendChild(ins);
    document.body.appendChild(tok);
    item.classList.add('is-source');
    document.documentElement.classList.add('sum-dragging');
    v.drag = { item: item, to: -1, ins: ins, tok: tok, x: e.clientX, y: e.clientY, sc: scroller(v.root) };
    v.dragOn = {
      move: function (ev) { var d = v.drag; if (!d) return; d.x = ev.clientX; d.y = ev.clientY; moveToken(d); placeLine(v); },
      up: function () { drop(v); },
      cancel: function () { endDrag(v); update(v); },
      key: function (ev) { if (ev.key === 'Escape' && v.drag) { ev.preventDefault(); ev.stopPropagation(); endDrag(v); update(v); } },
    };
    document.addEventListener('pointermove', v.dragOn.move);
    document.addEventListener('pointerup', v.dragOn.up);
    document.addEventListener('pointercancel', v.dragOn.cancel);
    window.addEventListener('keydown', v.dragOn.key, true);
    moveToken(v.drag);
    requestAnimationFrame(function loop() {
      var d = v.drag;
      if (!d) return;
      var sc = d.sc, r = sc === document.scrollingElement || sc === document.documentElement ? { top: 0, bottom: innerHeight } : sc.getBoundingClientRect(), dy = 0;
      if (d.y < r.top + 40) dy = -12; else if (d.y > r.bottom - 64) dy = 12;
      if (dy) { var was = sc.scrollTop; sc.scrollTop += dy; if (sc.scrollTop !== was) placeLine(v); }
      requestAnimationFrame(loop);
    });
  }
  function moveToken(d) { d.tok.style.left = Math.round(d.x - 14) + 'px'; d.tok.style.top = Math.round(d.y - 16) + 'px'; }
  function placeLine(v) {
    var d = v.drag, its = [].slice.call(v.body.children), from = its.indexOf(d.item), to = its.length;
    for (var i = 0; i < its.length; i++) { var r = its[i].getBoundingClientRect(); if (d.y < r.top + r.height / 2) { to = i; break; } }
    d.to = to;
    var same = to === from || to === from + 1;
    d.ins.hidden = same;
    if (same) return;
    var lr = v.body.getBoundingClientRect(), y;
    if (to === 0) y = its[0].getBoundingClientRect().top - 1;
    else if (to === its.length) y = its[its.length - 1].getBoundingClientRect().bottom + 1;
    else y = (its[to - 1].getBoundingClientRect().bottom + its[to].getBoundingClientRect().top) / 2;
    d.ins.style.left = Math.round(lr.left) + 'px';
    d.ins.style.width = Math.round(lr.width) + 'px';
    d.ins.style.top = Math.round(y) + 'px';
  }
  function drop(v) {
    var d = v.drag;
    if (!d) return;
    var ok = d.to >= 0 && !d.ins.hidden, ref = ok ? v.body.children[d.to] || null : null;
    endDrag(v);
    if (ok && d.item.parentNode === v.body) { v.body.insertBefore(d.item, ref); commit(v); }
    update(v);
  }
  function endDrag(v) {
    var d = v.drag;
    if (!d) return;
    document.removeEventListener('pointermove', v.dragOn.move);
    document.removeEventListener('pointerup', v.dragOn.up);
    document.removeEventListener('pointercancel', v.dragOn.cancel);
    window.removeEventListener('keydown', v.dragOn.key, true);
    d.item.classList.remove('is-source');
    d.ins.remove();
    d.tok.remove();
    document.documentElement.classList.remove('sum-dragging');
    v.drag = null;
  }

  /* ---------- events ---------- */
  function bind(v) {
    var root = v.root;
    function on(type, fn) { root.addEventListener(type, function (e) { if (!v.dead) fn(e); }); }
    on('click', function (e) {
      var b = e.target.closest('button'), item = e.target.closest('.sum-item');
      if (b && b.getAttribute('data-act') === 'report') { if (v.o.onReport) v.o.onReport(); return; }
      if (!item || !v.body.contains(item)) return;
      if (b) {
        var act = b.getAttribute('data-act'), row = b.closest('.sum-row'), t = row && row._t;
        if (!t || b.disabled) return;
        if (act === 'trace') openTrace(v, item);
        else if (act === 'pause' || act === 'remove') run(v, act, t);
        return;
      }
      select(v, v.sel === item._key ? null : item._key, true);
    });
    on('dblclick', function (e) {
      if (e.target.closest('button')) return;
      var item = e.target.closest('.sum-item');
      if (item && v.body.contains(item)) openTrace(v, item);
    });
    on('keydown', function (e) {
      if (e.defaultPrevented) return;
      var g = e.target.closest && e.target.closest('.sum-grip');
      if (g && e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) { e.preventDefault(); nudge(v, g.closest('.sum-item'), e.key === 'ArrowUp' ? -1 : 1); return; }
      var it = e.target.classList && e.target.classList.contains('sum-item') ? e.target : null;
      if (!it) return;
      if (e.key === 'Enter') { e.preventDefault(); openTrace(v, it); }
      else if (e.key === ' ') { e.preventDefault(); select(v, v.sel === it._key ? null : it._key, true); }
    });
    // A screen that still runs its own shared drag handles the grip first (it prevents the default).
    on('pointerdown', function (e) {
      if (e.button !== 0 || e.defaultPrevented || v.drag) return;
      var g = e.target.closest && e.target.closest('.sum-grip');
      if (!g || !v.body.contains(g)) return;
      e.preventDefault();
      startDrag(v, e, g);
    });
    on('pointerover', function (e) {
      var st = !v.drag && e.target.closest && e.target.closest('.sum-st');
      if (st && v.body.contains(st)) showTip(v, st.parentNode);
    });
    on('pointerout', function (e) {
      var st = e.target.closest && e.target.closest('.sum-st');
      if (st && !(e.relatedTarget && st.contains(e.relatedTarget))) hideTip(v);
    });
    on('focusin', function (e) {
      var it = e.target.classList && e.target.classList.contains('sum-item') ? e.target : null;
      if (it && it.matches(':focus-visible')) showTip(v, it.querySelector('.sum-row')); else hideTip(v);
    });
    on('focusout', function () { hideTip(v); });
  }

  function mount(root, opts) {
    if (root._sum) root._sum.destroy();
    var v = { root: root, o: opts || {}, items: {}, sel: null, drag: null, pend: null, over: {}, tip: null, c: null, dead: false };
    build(v);
    bind(v);
    var api = {
      el: root,
      update: function () { update(v); },
      // Pick a row from outside (a timeline card was picked); takes any row id of the item, null clears.
      select: function (id) { select(v, keyOf(v, id), false); },
      selected: function () { return idOf(v, v.sel); },
      // The order changed elsewhere (the stack): show it at once, the next snapshot confirms it.
      setOrder: function (ids) { v.pend = { ids: (ids || []).slice(), until: Date.now() + HOLD_MS }; update(v); },
      order: function () { return domOrder(v); },
      destroy: function () {
        endDrag(v);
        hideTip(v);
        v.dead = true;
        root.innerHTML = '';
        delete root._sum;
      },
    };
    root._sum = api;
    if (v.o.live !== false && window.Live && window.Live.onData) window.Live.onData(function () { update(v); });
    update(v);
    return api;
  }

  window.SUM = { mount: mount };
})();
