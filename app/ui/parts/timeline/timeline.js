// Whilo Trace v2, part "timeline": the timeline card, the stack of cards and their live behaviour.
// Data only from window.WT (base/wt.js): rows of WT.targets with t.stats (t.rangeStats for a picked period),
// time from WT.fromMs, WT.toMs and WT.bucketMs (bucket i starts at WT.fromMs + i * WT.bucketMs).
// Every visible text has data-en: theme.js swaps it at load; text built later (hover, selection, drag,
// live updates) is written in the current language right away.
// Status of a track: the core's (t.status) for the window, rangeStats.status for a picked period. A paused track
// gets a «paused» chip next to its status. A row with t.error shows a short error status and, in place of its
// chart, the error well of the history part (texts from HE.errorInfo, the one place for error texts).
//
// window.WTT:
//   view(el, items, o) -> a live stack. items: from fromTargets(), or null to take them from WT.targets on every update.
//     o.stack (head and shared time axis), o.select (pick a period with the mouse), o.reorder (drag the grip, Alt+arrow),
//     o.layout 'side' | 'row', o.h and o.hj (plot and jitter heights), o.mode 'tracks' | 'overlay' | 'switch',
//     o.rangeMs [fromMs, toMs) or o.range [i0, i1]: the period picked at start.
//     Callbacks: o.onRange([fromMs, toMs) | null, [i0, i1] | null) when the user picks or drops a period,
//     o.onOrder(rowIds, itemIds) after a drag, o.onUnpin(rowId, itemId) from the button on a pinned hop.
//   The view: update(items?, o?) after new data, render(), setRange([i0, i1] | null), setRangeMs([fromMs, toMs] | null)
//     (both silent: no onRange), range (bucket indices of the picked period), rangeMs, items, byId, o, destroy().
//     Updates patch the page in place: hover, the picked period, a drag, focus and scrolling stay as they are.
//   fromTargets(rows?): targets with one track per probe type, pinned hops as cards of their own.
//   card, stackHtml, dragHtml: static markup; legendHtml(); tAt(i), tEnd(i): bucket times as text;
//   timeText(ms, sec), spanText([fromMs, toMs]): {ru, en}; statsOf(src, range); S, L, EN.
(function () {
  'use strict';
  var WT = window.WT, W = 1000;
  var EN = document.documentElement.dataset.lang === 'en';
  var CALM = window.matchMedia ? window.matchMedia('(prefers-reduced-motion: reduce)') : null;

  /* ---------- text ---------- */
  function esc(s) {
    return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }
  function L(ru, en) { return EN ? en : ru; }
  function S(ru, en, cls, attrs) {
    return '<span' + (cls ? ' class="' + cls + '"' : '') + (attrs || '') + ' data-en="' + esc(en) + '">' + esc(EN ? en : ru) + '</span>';
  }
  // WT.fmt gives «10,0» for 9.95...10: here it is «10», like every number from 10 up.
  function fr(v) { var s = WT.fmt(v); return s === '10,0' ? '10' : s; }
  function fe(v) { return fr(v).replace(',', '.'); }
  function MS(v, cls) { return S(fr(v) + ' мс', fe(v) + ' ms', cls); }
  // «5,8 %» in Russian (number and sign apart, as in the summary and the trace), «5.8%» in English.
  function PCT(v, cls) { return S(fr(v) + ' %', fe(v) + '%', cls); }
  function axn(v) { var s = String(Math.round(v * 10) / 10); return { ru: s.replace('.', ','), en: s }; }
  var NAME_EN = { 'роутер': 'router' };
  function nm(s) { return { ru: s, en: NAME_EN[s] || s }; }
  function f1(x) { return String(Math.round(x * 10) / 10); }
  function f2(x) { return String(Math.round(x * 100) / 100); }
  function probeOf(p) {
    var u = String(p || '').toUpperCase();
    return WT.probes[p] || { label: u, ru: u, en: u };
  }

  /* ---------- time: from the snapshot ---------- */
  function n() { return WT.N || 120; }
  function bucket() { return WT.bucketMs || (WT.toMs - WT.fromMs) / n() || 5000; }
  function msAt(i) { return WT.fromMs + i * bucket(); }
  function two(x) { return (x < 10 ? '0' : '') + x; }
  function dayStart(ms) { var d = new Date(ms); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); }
  // A moment that is not today gets its day in front: «вчера», or the date for older days in the format of the
  // whole screen (the history part's dateOf): 25.10.2026, in English 2026‑10‑25 with non-breaking hyphens.
  function dayOf(ms) {
    var ago = Math.round((dayStart(WT.nowMs || WT.toMs || Date.now()) - dayStart(ms)) / 864e5), d = new Date(ms);
    if (ago <= 0) return { ru: '', en: '' };
    if (ago === 1) return { ru: 'вчера ', en: 'yesterday ' };
    var y = d.getFullYear(), m = two(d.getMonth() + 1), dd = two(d.getDate());
    return { ru: dd + '.' + m + '.' + y + ' ', en: y + '‑' + m + '‑' + dd + ' ' };
  }
  // «15:54» or, with seconds, «15:54:05»; buckets of 2.5 s also show tenths: «15:54:02,5». pre: the day in front.
  function timeParts(ms, sec) {
    var d = new Date(ms), hm = two(d.getHours()) + ':' + two(d.getMinutes()), t = 0;
    if (sec) { hm += ':' + two(d.getSeconds()); t = Math.floor(d.getMilliseconds() / 100); }
    return { pre: dayOf(ms), t: { ru: hm + (t ? ',' + t : ''), en: hm + (t ? '.' + t : '') } };
  }
  function timeText(ms, sec) { var p = timeParts(ms, sec); return { ru: p.pre.ru + p.t.ru, en: p.pre.en + p.t.en }; }
  function durText(ms) {
    var s = Math.round(ms / 100) / 10, h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = Math.round((s % 60) * 10) / 10;
    var ru = [], en = [];
    if (h) { ru.push(h + ' ч'); en.push(h + ' h'); }
    if (m) { ru.push(m + ' мин'); en.push(m + ' min'); }
    if (r || (!h && !m)) { ru.push(String(r).replace('.', ',') + ' с'); en.push(r + ' s'); }
    return { ru: ru.join(' '), en: en.join(' ') };
  }
  // Bucket times with seconds while buckets are shorter than a minute («15:54:05»), else «15:54».
  function secs() { return bucket() < 60000; }
  // A period «15:54:00-15:56:00» and its length; the day is written once when both ends share it.
  function spanText(ms) {
    var a = timeText(ms[0], secs()), b = timeText(ms[1], secs()), p = dayOf(ms[1]);
    if (dayStart(ms[0]) === dayStart(ms[1])) b = { ru: b.ru.slice(p.ru.length), en: b.en.slice(p.en.length) };
    return { ru: a.ru + '-' + b.ru, en: a.en + '-' + b.en, dur: durText(ms[1] - ms[0]) };
  }
  function tAt(i) { var t = timeText(msAt(i), secs()); return L(t.ru, t.en); }
  function tEnd(i) { var t = timeText(msAt(i + 1), secs()); return L(t.ru, t.en); }
  // Window of the snapshot by its length («10m»); before the first snapshot, the one the screen asked for.
  function winKey(fallback) {
    var span = (WT.toMs - WT.fromMs) / 1000, ws = WT.windowSeconds || {};
    for (var k in ws) if (Math.abs(ws[k] - span) < 1) return k;
    return fallback || '10m';
  }
  function winOf(key) {
    var w = WT.windows.filter(function (x) { return x[0] === key; })[0] || WT.windows[1];
    return { ru: w[1], en: w[2] };
  }
  // Time axis: labels on round times (every 1, 2, 5, 10 min, 1 h, 4 h...), about six per window, each where its
  // moment is on the charts, so in live mode they ride along with the data.
  var STEPS = [15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 14400, 21600, 43200];
  function ticks() {
    var from = WT.fromMs, to = WT.toMs, span = to - from, out = [], st = STEPS[STEPS.length - 1] * 1000;
    if (!(span > 0)) return out;
    for (var k = 0; k < STEPS.length; k++) if (span / (STEPS[k] * 1000) <= 6 + 1e-9) { st = STEPS[k] * 1000; break; }
    var d0 = dayStart(from);
    for (var t = d0 + Math.ceil((from - d0) / st - 1e-9) * st; t <= to + 1e-6; t += st) {
      var p = timeParts(t, st < 60000);
      out.push({ ms: t, x: (t - from) / span * W, text: { ru: p.pre.ru + p.t.ru, en: p.pre.en + p.t.en }, pre: p.pre, hm: p.t, day: dayStart(t) });
    }
    return out;
  }

  /* ---------- statistics ---------- */
  function setOf(list) { var s = {}; (list || []).forEach(function (i) { s[i] = true; }); return s; }
  function ok(v) { return v != null && isFinite(v); }
  function num(v) { return ok(v) ? +v : null; }
  function vals(s) { return s && s.series ? WT.series(s.series) : []; }
  function hasReplies(s) { return vals(s).some(ok); }
  // Figures of a track: the window from the core (t.stats), the picked period from the core (t.rangeStats,
  // core = true once data for that period came) or, until then, counted here by the same rule as the mock.
  // status: the core's status for the period (rangeStats.status), when it sends one.
  function statsOf(src, range, core) {
    if (!src || (!src.series && !src.stats)) return null;
    var r = range && core && src.rangeStats ? src.rangeStats : null;
    if (!range || r) {
      var w = r || WT.stats(src);
      return { lossPct: +w.lossPct || 0, avg: num(w.avg), min: num(w.min), max: num(w.max), status: r && r.status ? r.status : null };
    }
    var v = vals(src), loss = setOf(src.loss), part = setOf(src.partial), seen = [], lost = 0, cnt = 0;
    for (var i = range[0]; i <= range[1]; i++) {
      cnt++;
      if (loss[i]) lost++;
      if (!loss[i] && !part[i] && ok(v[i])) seen.push(v[i]);
    }
    var sum = seen.reduce(function (a, b) { return a + b; }, 0);
    return {
      lossPct: cnt ? lost / cnt * 100 : 0, status: null,
      avg: seen.length ? sum / seen.length : null,
      min: seen.length ? Math.min.apply(null, seen) : null,
      max: seen.length ? Math.max.apply(null, seen) : null,
    };
  }
  // Status of a track. The window: always the core's (t.status), never counted again here. A picked period
  // (ranged): the core's status for it (rangeStats.status); until the core sends one, «loss» and «OK» follow the
  // core's rule (loss from LOSS_NOTABLE_PCT) and the other statuses stay. A row with an error: 'error'.
  var LOSS_NOTABLE_PCT = 2;
  function keyOf(tr, st, ranged) {
    if (errRow(tr)) return 'error';
    var k = tr.status;
    if (ranged && st) {
      if (st.status && WT.statuses[st.status]) k = st.status;
      else if (k === 'loss' || k === 'ok') k = st.lossPct >= LOSS_NOTABLE_PCT ? 'loss' : 'ok';
    }
    return WT.statuses[k] ? k : 'ok';
  }
  // Red only for loss: «no reply» and a row error get the neutral grey mark, as in the summary and the trace.
  var MARK = { ok: 'ok', loss: 'loss', partial: 'partial', silent: 'silent', noreply: 'muted', error: 'muted' };
  function mark(k) { return '<i class="mark" style="background:var(--' + (MARK[k] || 'ok') + ')"></i>'; }

  /* ---------- row errors: texts from the history part (HE.errorInfo), a short fallback without it ---------- */
  function errRow(tr) { var s = tr && tr.src; return s && s.error ? s : null; }
  // A text of HE.errorInfo in the interface language: a string, or [ru, en].
  function inLang(v) { return Array.isArray(v) ? String(v[EN ? 1 : 0] || v[0] || '') : v == null ? '' : String(v); }
  function errInfo(t) {
    var he = window.HE, code = String(t.error), base = /^([a-z_]+)\s*:/.exec(code);
    function ask(c) {
      var e = null;
      if (he && he.errorInfo) {
        try { e = he.errorInfo(c, { host: t.host, probe: t.probe, interval: t.interval }); } catch (x) { e = null; }
      }
      return e || {};
    }
    var e = ask(code), status = inLang(e.status), title = inLang(e.title), hint = inLang(e.hint), text = inLang(e.text);
    // «probe_unavailable: <detail>»: without a status for the whole code, the short status of its base code
    if (!status && base) status = inLang(ask(base[1]).status);
    if (!title) { title = text; hint = ''; }
    if (!title) title = L('Ошибка: ', 'Error: ') + code;
    return { status: status || L('ошибка', 'Error'), title: title, hint: hint };
  }
  // The place of the chart: the error well of the history part (HE.rowError), or the same well with the texts only.
  function wellHtml(t, e) {
    var he = window.HE, body = null;
    if (he && he.rowError) {
      try { body = he.rowError(t).body; } catch (x) { body = null; }
    }
    if (typeof body === 'string' && body.indexOf('er-well') >= 0) return body;
    return '<div class="er-well"><div class="er-msg"><div>' + S(e.title, e.title, 'er-msg-t') +
      (e.hint ? S(e.hint, e.hint, 'er-msg-s') : '') + '</div></div></div>';
  }
  // Buckets a line may step over: when rounds are rarer than buckets (interval longer than a bucket), the buckets
  // between two rounds are empty, not lost.
  function bridgeOf(s) { return Math.max(0, Math.ceil(((+(s && s.interval) || 0) * 1000) / bucket() - 1e-9)); }
  function hit(set, a, b) { for (var i = a; i <= b; i++) if (set[i]) return true; return false; }
  // Probes in bucket i: exact when the row has per-bucket counts from the core (sentN: probes sent, lostN: probes
  // without a reply), else from the interval. k is how many got no reply; null when only «some» is known.
  function countsAt(s, i, miss, v) {
    var sn = s.sentN ? num(s.sentN[i]) : null, ln = s.lostN ? num(s.lostN[i]) : null;
    if (sn != null && sn > 0 && ln != null) return { n: sn, k: Math.min(ln, sn) };
    var c = Math.max(1, Math.round(bucket() / ((+s.interval || 2.5) * 1000)));
    if (!miss) return { n: c, k: 0 };
    if (!ok(v)) return { n: c, k: c };
    return { n: c, k: c <= 2 ? 1 : null };
  }

  /* ---------- chart geometry: bucket i takes [i, i + 1) / N of the width, its point is in the middle ---------- */
  function BW() { return W / n(); }
  function X(i) { return (i + 0.5) * BW(); }
  function inside(i) { return i >= 0 && i < n(); }
  function runs(idx) {
    var a = (idx || []).filter(inside).sort(function (p, q) { return p - q; }), out = [];
    a.forEach(function (i) {
      var r = out[out.length - 1];
      if (r && i <= r[1] + 1) r[1] = Math.max(r[1], i); else out.push([i, i]);
    });
    return out;
  }
  // Loss: solid bars over the whole height; neighbouring buckets merge into one bar without seams.
  function barsPath(idx, H) {
    return runs(idx).map(function (r) {
      var a = r[0] * BW(), b = (r[1] + 1) * BW();
      return 'M' + f1(a) + ' 0H' + f1(b) + 'V' + H + 'H' + f1(a) + 'Z';
    }).join('') || 'M0 0';
  }
  // Partial replies: short marks at the bottom, one per bucket.
  function marksPath(idx, H) {
    var mw = BW() * 0.5, mh = H >= 90 ? 8 : 6;
    return (idx || []).filter(inside).map(function (i) {
      return 'M' + f1(X(i) - mw / 2) + ' ' + (H - mh) + 'h' + f1(mw) + 'v' + mh + 'h-' + f1(mw) + 'Z';
    }).join('') || 'M0 0';
  }
  function rangeX(a, b) { return [a * BW(), (b + 1) * BW()]; }
  function rangePath(a, b, H) { var x = rangeX(a, b); return 'M' + f1(x[0]) + ' 0H' + f1(x[1]) + 'V' + H + 'H' + f1(x[0]) + 'Z'; }
  function edgePath(a, b, H) { var x = rangeX(a, b); return 'M' + f1(x[0]) + ' 0V' + H + 'M' + f1(x[1]) + ' 0V' + H; }
  function yOf(v, max, H) { return H - Math.min(v, max) / max * H; }
  // Latency goes through the replies only: a bucket where nothing answered (null) breaks the line, a lone reply is a dot.
  function linePath(s, max, H) {
    var v = vals(s), miss = setOf((s.loss || []).concat(s.partial || [])), gap = bridgeOf(s), N = n(), d = '', seg = [], last = -1;
    function flush() {
      if (seg.length) d += 'M' + seg[0] + (seg.length === 1 ? 'h0.1' : 'L' + seg.slice(1).join('L'));
      seg = [];
    }
    for (var i = 0; i < N; i++) {
      if (!ok(v[i])) continue;
      if (last >= 0 && !(i - last - 1 <= gap && !hit(miss, last + 1, i - 1))) flush();
      seg.push(f1(X(i)) + ' ' + f1(yOf(v[i], max, H)));
      last = i;
    }
    flush();
    return d || 'M0 0';
  }
  // Jitter: how much the latency changed from the previous reply, scaled to the strip; breaks where the line breaks.
  function jitterPath(s, H) {
    var v = vals(s), miss = setOf((s.loss || []).concat(s.partial || [])), gap = bridgeOf(s), N = n(), pts = [], top = 0, last = -1;
    for (var i = 0; i < N; i++) {
      if (!ok(v[i])) continue;
      var j = last >= 0 && i - last - 1 <= gap && !hit(miss, last + 1, i - 1) ? Math.abs(v[i] - v[last]) : null;
      pts.push([i, j]);
      if (j != null && j > top) top = j;
      last = i;
    }
    top = Math.max(0.001, top);
    var d = '', pen = false;
    pts.forEach(function (p) {
      if (p[1] == null) { pen = false; return; }
      d += (pen ? 'L' : 'M') + f1(X(p[0])) + ' ' + f1(H - 1 - p[1] / top * (H - 3));
      pen = true;
    });
    return d || 'M0 0';
  }
  function yTicks(max, H) {
    var steps = [0.5, 1, 2, 2.5, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000], lim = H >= 90 ? 3 : 2, s = max;
    for (var k = 0; k < steps.length; k++) if (max / steps[k] <= lim + 1e-9) { s = steps[k]; break; }
    var out = [];
    for (var v = 0; v <= max + 1e-9; v += s) out.push(Math.round(v * 100) / 100);
    var top = out[out.length - 1];
    // The top of the scale is labelled only when it is at least 24 px from the label below.
    if (max - top > 1e-9 && (max - top) / max * H >= 24) out.push(max);
    return out;
  }
  function gridY(max, H, has) {
    var d = '';
    if (has) yTicks(max, H).forEach(function (v) {
      var y = yOf(v, max, H);
      if (y > 0.5 && y < H - 0.5) d += 'M0 ' + f1(y) + 'H' + W;
    });
    return d || 'M0 0';
  }
  function gridX(tk, H) {
    return (tk || []).filter(function (t) { return t.x > 0.5 && t.x < W - 0.5; })
      .map(function (t) { return 'M' + f1(t.x) + ' 0V' + H; }).join('') || 'M0 0';
  }
  function thresholds() {
    var s = WT.settings || {}, w = +s.warnMs || 100, b = +s.badMs || 200;
    return { warn: w, bad: b > w ? b : w * 2 };
  }
  function svg(H, inner) {
    return '<svg viewBox="0 0 ' + W + ' ' + H + '" preserveAspectRatio="none" aria-hidden="true">' + inner + '</svg>';
  }
  // Paths marked mv move with time: they slide when a live update shifts the window.
  function P(cls, d) { return '<path class="' + cls + '" d="' + d + '"/>'; }
  var DOT = '<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><circle class="ring" cx="6" cy="6" r="6"/><circle class="core" cx="6" cy="6" r="4"/></svg>';
  var GRIP = '<svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" aria-hidden="true"><path d="M5.5 3.5h0M10.5 3.5h0M5.5 8h0M10.5 8h0M5.5 12.5h0M10.5 12.5h0"/></svg>';
  var RANGE = '<svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 3v12M15 3v12M3 9h12M6 6.5 3.5 9 6 11.5M12 6.5 14.5 9 12 11.5"/></svg>';
  var CLOSE = '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/></svg>';

  /* ---------- pieces of the label ---------- */
  function chipHtml(probe) { var p = probeOf(probe); return S(p.ru, p.en, 'chip'); }
  function gripHtml() {
    var t = L('Перетащите или нажмите Alt+стрелку, чтобы поменять порядок', 'Drag, or press Alt+arrow, to reorder');
    return '<button class="tl-grip" type="button" aria-label="' + esc(t) + '" title="' + esc(t) + '">' + GRIP + '</button>';
  }
  // Unpin a hop from the main screen: on the label of a pinned hop, shown under the pointer and with the keyboard.
  function pinnedRow(item) {
    var src = item.tracks && item.tracks[0] && item.tracks[0].src;
    if (!item.pinned && !(src && src.hopOf)) return null;
    return item.rows ? item.rows[0] : (src && src.id) || item.id;
  }
  function unpinHtml(item, o) {
    var row = o.unpin ? pinnedRow(item) : null;
    if (row == null) return '';
    var t = L('Убрать хоп с главного экрана', 'Unpin the hop from the main screen');
    return '<button class="tl-unpin" type="button" data-row="' + esc(row) + '" aria-label="' + esc(t) + '" title="' + esc(t) + '">' + CLOSE + '</button>';
  }
  // Status of track tr with its key k (keyOf). The explanation is only in the hint; a target's «loss» has none
  // (the hint speaks of later hops, and the figures say the rest), as in the summary. A row error: its short
  // status, no hint.
  function statusHtml(tr, st, k, o) {
    var pz = !!(tr.src && tr.src.paused), name;
    if (k === 'error') {
      var e = errInfo(tr.src);
      return '<span class="tl-status is-err"' + (pz ? ' title="' + esc(e.status) + '"' : '') + '>' + mark(k) + S(e.status, e.status) + '</span>';
    }
    var s = WT.statuses[k], p = probeOf(tr.probe).label, hop = !!(tr.src && tr.src.hopOf);
    var hr = !hop && s.hint_target_ru ? s.hint_target_ru : s.hint_ru || '', he = !hop && s.hint_target_en ? s.hint_target_en : s.hint_en || '';
    var hint = k === 'loss' && !hop ? '' : (EN ? he : hr).replace('{probe}', p);
    name = L(s.ru, s.en).replace('{probe}', p);
    // next to the «paused» chip a long status may be cut short (timeline.css): its hint starts with the full name
    if (pz && hint) hint = name.charAt(0).toUpperCase() + name.slice(1) + '. ' + hint;
    var h = '<span class="tl-status"' + (hint ? ' tabindex="0" data-hint="' + esc(hint) + '"' : pz ? ' title="' + esc(name) + '"' : '') + '>' + mark(k) +
      S(s.ru.replace('{probe}', p), s.en.replace('{probe}', p));
    if (k === 'loss' && st) h += PCT(st.lossPct, 'num tl-pct');
    if (o && o.hint && hint) h += '<span class="tl-tip tl-hint">' + S(hr.replace('{probe}', p), he.replace('{probe}', p)) + '</span>';
    return h + '</span>';
  }
  // A paused track: the chip next to its status.
  function pauseHtml(tr) { return tr.src && tr.src.paused ? S('пауза', 'paused', 'chip tl-pz') : ''; }
  function statKV(kru, ken, vHtml) { return '<span class="tl-stat">' + S(kru, ken, 'k') + vHtml + '</span>'; }
  function statsHtml(st, k, o, compact) {
    if (k === 'silent' || k === 'error') return '';
    if (!st || st.avg == null) {
      return k === 'noreply' ? '<span class="tl-stats">' + statKV('потери', 'loss', PCT(st ? st.lossPct : 100, 'v num')) + '</span>' : '';
    }
    var pre = o.range ? S('за период', 'for period', 'tl-per') : '';
    if (compact) {
      return '<span class="tl-stats">' +
        statKV('сред.', 'avg', S(fr(st.avg), fe(st.avg), 'v num')) +
        statKV('мин.', 'min', S(fr(st.min), fe(st.min), 'v num')) +
        statKV('макс.', 'max', MS(st.max, 'v num')) + '</span>';
    }
    return '<span class="tl-stats">' + pre +
      statKV('сред.', 'avg', MS(st.avg, 'v num')) +
      statKV('мин.', 'min', MS(st.min, 'v num')) +
      statKV('макс.', 'max', MS(st.max, 'v num')) + '</span>';
  }
  function winHtml(key) { var w = winOf(key); return S(w.ru, w.en, 'tl-win'); }
  function hopSub(item, short) {
    if (!item.hop) return '';
    if (short) {
      var full = L('хоп ' + item.hop + ' на пути к ' + item.of, 'hop ' + item.hop + ' on the path to ' + item.of);
      return S('хоп ' + item.hop, 'hop ' + item.hop, 'tl-sub', ' title="' + esc(full) + '"');
    }
    if (item.hopTitle) return S('на пути к ' + item.of, 'on the path to ' + item.of, 'tl-sub');
    return S('хоп ' + item.hop + ' на пути к ' + item.of, 'hop ' + item.hop + ' on the path to ' + item.of, 'tl-sub');
  }
  function addrHtml(item) {
    return item.addr ? S(item.addr.ru, item.addr.en, 'tl-addr' + (item.addrMono ? ' mono' : '')) : '';
  }
  // Each label is centred on its moment; near the ends of the chart it shifts to stay inside (timeline.css).
  // A label on the same day as the one before keeps that day apart (.tl-xd): a tight axis hides it (View.fitAxis).
  function xAxisHtml(tk, tight) {
    tk = tk || ticks();
    return '<div class="tl-xax' + (tight ? ' is-tight' : '') + '">' + tk.map(function (t, k) {
      var p = f2(t.x / W * 100), cls = 'tl-xl num' + (t.x < 0.5 ? ' is-l' : t.x > W - 0.5 ? ' is-r' : ''), at = ' style="left:' + p + '%;--p:' + p + '"';
      if (t.pre && t.pre.ru && k > 0 && tk[k - 1].day === t.day) {
        return '<span class="' + cls + ' is-rep"' + at + '>' + S(t.pre.ru, t.pre.en, 'tl-xd') + S(t.hm.ru, t.hm.en) + '</span>';
      }
      return S(t.text.ru, t.text.en, cls, at);
    }).join('') + '</div>';
  }
  function yLabels(max, H) {
    var t = yTicks(max, H), top = t[t.length - 1];
    return t.map(function (v) {
      var y = yOf(v, max, H) / H * 100, a = axn(v), cls = 'tl-yl num' + (v === 0 ? ' b' : (v === top ? ' t' : ''));
      return v === top ? S(a.ru + ' мс', a.en + ' ms', cls, ' style="top:' + f2(y) + '%"') : S(a.ru, a.en, cls, ' style="top:' + f2(y) + '%"');
    }).join('');
  }

  /* ---------- a track: jitter strip and chart ---------- */
  function modeOf(item, o) { return item.tracks.length > 1 ? (o.mode || 'tracks') : 'single'; }
  function viewTracks(item, o, k) {
    var m = modeOf(item, o);
    if (m === 'overlay') return item.tracks.filter(function (t) { return !errRow(t); });
    if (m === 'switch') return [item.tracks[o.active || 0]];
    return [item.tracks[k || 0]];
  }
  // The body of a track: its chart, or the error well where a row has an error (no ms scale, no hover there).
  // The well takes the height of the jitter strip and the chart, so the stack keeps its rhythm.
  function bodyHtml(item, trs, k, o, overlay) {
    var live = trs.filter(function (t) { return !errRow(t); });
    if (live.length) return trackBody(item, live, k, o, overlay);
    var t = trs[0].src, e = errInfo(t), full = e.title + (e.hint ? ' ' + e.hint : '');
    return '<div class="tl-eb" title="' + esc(full) + '" style="height:calc(' + (o.h + o.hj) + 'px + var(--s1))">' + wellHtml(t, e) + '</div>';
  }
  function plotMax(trs) { return Math.max.apply(null, trs.map(function (t) { return +t.src.max || 10; })); }
  function plotSvg(trs, H, sel, overlay, tk) {
    var srcs = trs.map(function (t) { return t.src; }), max = plotMax(trs), th = thresholds(), loss = [];
    var has = srcs.some(hasReplies), silent = trs.length === 1 && trs[0].status === 'silent';
    srcs.forEach(function (s) { loss = loss.concat(s.loss || []); });
    // Background by the latency thresholds from the settings; a chart without replies keeps the plain well.
    var d = P('sel mv', sel ? rangePath(sel[0], sel[1], H) : 'M0 0') +
      P('z-warn', has && max > th.warn ? WT.path.zone(th.warn, th.bad, max, W, H) : 'M0 0') +
      P('z-bad', has && max > th.bad ? WT.path.zone(th.bad, 1e9, max, W, H) : 'M0 0') +
      P('grid', gridY(max, H, has)) +
      P('hatch', silent ? WT.path.hatch(W, H, 14) : 'M0 0') +
      P('grid gx mv', gridX(tk, H)) +
      P('loss mv', barsPath(loss, H));
    srcs.forEach(function (s) { d += P('part mv', marksPath(s.partial, H)); });
    srcs.forEach(function (s, k) { d += P('lat mv' + (overlay && k ? ' l2' : ''), linePath(s, max, H)); });
    return svg(H, d + P('sel-edge mv', sel ? edgePath(sel[0], sel[1], H) : 'M0 0'));
  }
  function trackBody(item, trs, k, o, overlay) {
    var H = o.h, HJ = o.hj, sel = o.range || o.dragSel || null;
    var paused = trs.every(function (t) { return t.src && t.src.paused; });
    // The ms scale is there only when something answered (no scale for «all lost» and for the hatch).
    var has = trs.some(function (t) { return hasReplies(t.src); });
    var j = P('sel mv', sel ? rangePath(sel[0], sel[1], HJ) : 'M0 0');
    trs.forEach(function (t, m) { j += P('jit mv' + (overlay ? (m ? ' l2' : ' l1') : ''), jitterPath(t.src, HJ)); });
    return '<div class="tl-tb' + (paused ? ' is-paused' : '') + '">' +
      '<div class="tl-jrow">' + S('джиттер', 'jitter', 'tl-jlab') + '<div class="tl-jwell" style="height:' + HJ + 'px">' + svg(HJ, j) + '</div></div>' +
      '<div class="tl-prow"><div class="tl-yax">' + (has ? yLabels(plotMax(trs), H) : '') + '</div>' +
      '<div class="tl-plot' + (o.live ? ' is-live' : '') + '" data-k="' + k + '" style="height:' + H + 'px">' +
      '<div class="tl-clip">' + plotSvg(trs, H, sel, overlay, o.tk) + '</div>' +
      '<div class="tl-ov">' + frozenOverlay(item, k, o) + '</div></div></div></div>';
  }

  /* ---------- hover and labels over the chart ---------- */
  // The bucket to show for a hover at i: i itself, or the nearest one with data when i is empty only because
  // rounds are rarer than buckets.
  function snap(s, i) {
    var v = vals(s), miss = setOf((s.loss || []).concat(s.partial || [])), b = bridgeOf(s);
    if (ok(v[i]) || miss[i]) return i;
    for (var d = 1; d <= b; d++) {
      if (i - d >= 0 && (ok(v[i - d]) || miss[i - d])) return i - d;
      if (i + d < n() && (ok(v[i + d]) || miss[i + d])) return i + d;
    }
    return i;
  }
  function cellOf(tr, i) {
    var s = tr.src, p = probeOf(tr.probe).label;
    if (errRow(tr)) { var e = errInfo(s); return { silent: S(e.status, e.status), mk: 'error' }; }
    if (tr.status === 'silent') {
      var st = WT.statuses.silent;
      return { silent: S(st.ru.replace('{probe}', p), st.en.replace('{probe}', p)), mk: 'silent' };
    }
    var v = vals(s)[i], lost = !!setOf(s.loss)[i], part = !lost && !!setOf(s.partial)[i];
    var lat = ok(v) ? MS(v, 'v num') : lost || part ? S('нет ответа', 'no reply', 'v') : S('нет данных', 'no data', 'v');
    if (!lost && !part && !ok(v)) return { lat: lat, lk: null, lv: '' };
    var c = countsAt(s, i, lost || part, v);
    function share(k) { return k == null ? S('часть из ' + c.n, 'some of ' + c.n) : S(k + ' из ' + c.n, k + ' of ' + c.n); }
    return {
      lat: lat,
      lk: part ? { ru: 'Ответы', en: 'Replies' } : { ru: 'Потери', en: 'Loss' },
      lv: lost ? '<span class="v num">' + mark('loss') + share(c.k) + '</span>'
        : part ? '<span class="v num">' + mark('partial') + share(c.k == null ? null : c.n - c.k) + '</span>'
        : S(c.k + ' из ' + c.n, c.k + ' of ' + c.n, 'v num'),
    };
  }
  function tipBody(item, i, o) {
    var m = modeOf(item, o), list = m === 'switch' ? [item.tracks[o.active || 0]] : item.tracks;
    var t = timeText(msAt(i), secs()), h = S(t.ru, t.en, 'tl-tip-t num');
    if (list.length === 1) {
      var c = cellOf(list[0], i);
      if (c.silent) return h + '<div class="tl-tip-s">' + mark(c.mk) + c.silent + '</div>';
      return h + '<div class="tl-tip-g">' + S('Задержка', 'Latency') + c.lat + (c.lk ? S(c.lk.ru, c.lk.en) + c.lv : '') + '</div>';
    }
    h += '<div class="tl-tip-g g3"><span></span>' + S('задержка', 'latency', 'h') + S('потери', 'loss', 'h');
    list.forEach(function (tr, k) {
      var c = cellOf(tr, i), p = probeOf(tr.probe);
      h += '<span class="p">' + (m === 'overlay' ? '<i class="tl-key' + (k ? ' k2' : '') + '"></i>' : '') + S(p.ru, p.en) + '</span>';
      h += c.silent ? c.silent + '<span></span>' : c.lat + (c.lv || '<span></span>');
    });
    return h + '</div>';
  }
  function hoverLayer(item, k, i, o, isSrc) {
    var x = X(i) / W * 100, h = '<div class="tl-xh" style="left:' + f2(x) + '%"></div>';
    if (!isSrc) return h;
    var trs = viewTracks(item, o, k), max = plotMax(trs);
    // A tooltip about one track snaps to its nearest round; a tooltip that lists several tracks stays at i.
    var at = modeOf(item, o) === 'switch' || item.tracks.length === 1 ? snap(trs[0].src, i) : i;
    trs.forEach(function (tr, m) {
      var v = vals(tr.src)[at];
      if (!ok(v)) return;
      var y = yOf(v, max, o.h) / o.h * 100;
      h += '<span class="tl-dot' + (modeOf(item, o) === 'overlay' && m ? ' d2' : '') + '" style="left:' + f2(X(at) / W * 100) + '%;top:' + f2(y) + '%">' + DOT + '</span>';
    });
    var pos = x > 62 ? 'right:calc(' + f2(100 - x) + '% + 14px)' : 'left:calc(' + f2(x) + '% + 14px)';
    return h + '<div class="tl-tip" style="' + pos + ';top:6px">' + tipBody(item, at, o) + '</div>';
  }
  function selLabel(a, b) {
    var x = rangeX(a, b), mid = (x[0] + x[1]) / 2 / W * 100, t = spanText([msAt(a), msAt(b + 1)]);
    return '<div class="tl-tip tl-sellab num" style="left:' + f2(mid) + '%">' + S(t.ru + ' · ' + t.dur.ru, t.en + ' · ' + t.dur.en) + '</div>';
  }
  function frozenOverlay(item, k, o) {
    var h = '';
    if (o.hover) h += hoverLayer(item, k, o.hover.i, o, (o.hover.k || 0) === k);
    if (o.dragSel && (o.dragSelK == null || o.dragSelK === k)) h += selLabel(o.dragSel[0], o.dragSel[1]);
    return h;
  }

  /* ---------- card ---------- */
  function opts(o) {
    var d = { layout: 'row', h: 120, hj: 14, grip: true, win: false, axis: false, window: '10m', mode: 'tracks', active: 0 };
    for (var k in o || {}) d[k] = o[k];
    if (!d.tk) d.tk = ticks();
    return d;
  }
  function theadHtml(tr, st, key, o, k, overlay) {
    return '<div class="tl-thead' + (k === 0 ? ' is-first' : '') + '">' +
      (overlay ? '<i class="tl-key' + (k ? ' k2' : '') + '"></i>' : '') + chipHtml(tr.probe) + statusHtml(tr, st, key, null) + pauseHtml(tr) +
      '<span class="tl-sp"></span>' + statsHtml(st, key, o) + '</div>';
  }
  function switchHtml(item, ctx, act) {
    return '<div class="seg tl-seg" role="group" aria-label="' + esc(L('Тип пробы', 'Probe type')) + '">' + item.tracks.map(function (tr, k) {
      var key = ctx.keys[k], p = probeOf(tr.probe), st = WT.statuses[key], e = key === 'error' ? errInfo(tr.src) : null;
      return '<button type="button" data-k="' + k + '" aria-pressed="' + (k === act) + '">' + S(p.ru, p.en) + mark(key) +
        (e ? S(e.status, e.status, 'num') : key === 'loss' ? PCT(ctx.st[k] ? ctx.st[k].lossPct : 0, 'num')
          : S(st.ru.replace('{probe}', p.label), st.en.replace('{probe}', p.label), 'num')) + '</button>';
    }).join('') + '</div>';
  }
  function cardRow(item, o, ctx) {
    var trs = item.tracks, m = modeOf(item, o), act = m === 'switch' ? (o.active || 0) : 0;
    var h = '<div class="tl-head">' + (o.grip ? gripHtml() : '') + '<div class="tl-id">' +
      S(item.name.ru, item.name.en, 'tl-name') + addrHtml(item) + hopSub(item, false);
    if (m === 'single') h += chipHtml(trs[0].probe) + statusHtml(trs[0], ctx.st[0], ctx.keys[0], o) + pauseHtml(trs[0]);
    h += '</div>';
    if (m === 'switch') h += switchHtml(item, ctx, act) + pauseHtml(trs[act]);
    h += '<span class="tl-sp"></span>';
    if (m === 'single' || m === 'switch') h += statsHtml(ctx.st[act], ctx.keys[act], o);
    if (o.win) h += winHtml(o.win);
    h += unpinHtml(item, o) + '</div>';
    if (m === 'single') h += bodyHtml(item, [trs[0]], 0, o);
    else if (m === 'switch') h += bodyHtml(item, [trs[act]], act, o);
    else if (m === 'overlay') {
      trs.forEach(function (tr, k) { h += theadHtml(tr, ctx.st[k], ctx.keys[k], o, k, true); });
      h += bodyHtml(item, trs, 0, o, true);
    } else {
      trs.forEach(function (tr, k) { h += theadHtml(tr, ctx.st[k], ctx.keys[k], o, k, false) + bodyHtml(item, [tr], k, o); });
    }
    if (o.axis) h += xAxisHtml(o.tk);
    return h;
  }
  function cardSide(item, o, ctx) {
    var trs = item.tracks, h = '';
    trs.forEach(function (tr, k) {
      var side = '';
      if (k === 0) {
        side += '<div class="tl-l1">' + (o.grip ? gripHtml() : '') + S(item.name.ru, item.name.en, 'tl-name') + hopSub(item, true) + unpinHtml(item, o) + '</div>';
        if (item.addr) side += '<div class="tl-l2">' + addrHtml(item) + '</div>';
      }
      side += '<div class="tl-l3' + (tr.src && tr.src.paused ? ' has-pz' : '') + '">' + chipHtml(tr.probe) + statusHtml(tr, ctx.st[k], ctx.keys[k], o) + pauseHtml(tr) + '</div>';
      side += '<div class="tl-l4">' + statsHtml(ctx.st[k], ctx.keys[k], o, true) + '</div>';
      h += '<div class="tl-srow' + (k ? ' is-next' : '') + '"><div class="tl-side">' + side + '</div><div class="tl-main">' +
        bodyHtml(item, [tr], k, o) + (o.axis && k === trs.length - 1 ? xAxisHtml(o.tk) : '') + '</div></div>';
    });
    return h;
  }
  function card(item, o) {
    o = opts(o);
    var trs = item.tracks, ctx = { st: trs.map(function (tr) { return errRow(tr) ? null : statsOf(tr.src, o.range, o.core); }) };
    ctx.keys = trs.map(function (tr, k) { return keyOf(tr, ctx.st[k], !!o.range); });
    var inner = o.layout === 'side' && modeOf(item, o) !== 'overlay' && modeOf(item, o) !== 'switch' ? cardSide(item, o, ctx) : cardRow(item, o, ctx);
    var lay = inner.indexOf('tl-srow') >= 0 ? 'side' : 'row';
    return '<article class="tl tl--' + lay + (o.lifted ? ' is-lifted' : '') + '" data-id="' + esc(item.id) + '">' + inner + '</article>';
  }

  /* ---------- stack ---------- */
  function stackHead(rangeMs, win) {
    if (rangeMs) {
      var t = spanText(rangeMs);
      return '<div class="tl-shead is-sel" role="status">' + RANGE + S('Выделен период', 'Selected period', 'tl-sh-t') +
        S(t.ru, t.en, 'num tl-sh-r') + S('(' + t.dur.ru + ')', '(' + t.dur.en + ')', 'tl-sh-d') +
        S('Сводка и цифры таймлайнов посчитаны за этот период', 'The summary and timeline figures are for this period', 'tl-sh-n') +
        '<button class="btn tl-reset" type="button">' + S('Сбросить', 'Reset') + S('Esc', 'Esc', 'tl-esc') + '</button></div>';
    }
    var w = winOf(win);
    return '<div class="tl-shead"><h2>' + S('Таймлайны', 'Timelines') + '</h2>' + S('окно ' + w.ru + ', ось времени общая', w.en + ' window, shared time axis', 'tl-sh-w') +
      S('Выделите участок мышью, чтобы посчитать сводку за этот период', 'Drag across a chart to get the summary for that period', 'tl-sh-h') + '</div>';
  }
  function rangeMsOf(r) { return r ? [msAt(r[0]), msAt(r[1] + 1)] : null; }
  function stackHtml(items, o) {
    o = opts(o);
    return '<div class="tl-stackwrap" data-layout="' + o.layout + '">' + (o.head !== false ? stackHead(rangeMsOf(o.range), winKey(o.window)) : '') +
      '<div class="tl-stack">' + items.map(function (it) { return card(it, o); }).join('') + '</div>' +
      (o.sharedAxis !== false ? '<div class="tl-axisbar">' + xAxisHtml(o.tk) + '</div>' : '') + '</div>';
  }
  // A frozen drag for the part's page: the stack opens at the drop place (blue line on top), the card is lifted over it.
  function dragHtml(items, lifted, at, o) {
    o = opts(o);
    var h = '', lo = {}, k;
    for (k in o) lo[k] = o[k];
    lo.lifted = true;
    items.forEach(function (it, i) {
      if (i === at) h += '<div class="tl-slot">' + card(lifted, lo) + '</div>';
      h += card(it, o);
    });
    return '<div class="tl-stackwrap" data-layout="' + o.layout + '"><div class="tl-stack">' + h + '</div></div>';
  }

  /* ---------- live update in place ---------- */
  // Classes others put on the part's elements stay through an update: the screen's (scr-*) and the drag's.
  function keepClasses(a, cls) {
    var own = ' ' + cls + ' ', extra = (a.getAttribute('class') || '').split(/\s+/).filter(function (c) {
      return c && (c.indexOf('scr-') === 0 || c === 'is-source') && own.indexOf(' ' + c + ' ') < 0;
    });
    return extra.length ? (cls ? cls + ' ' : '') + extra.join(' ') : cls;
  }
  function syncAttrs(a, b) {
    var i, at, v;
    for (i = 0; i < b.attributes.length; i++) {
      at = b.attributes[i];
      v = at.name === 'class' ? keepClasses(a, at.value) : at.value;
      if (a.getAttribute(at.name) !== v) a.setAttribute(at.name, v);
    }
    for (i = a.attributes.length - 1; i >= 0; i--) {
      at = a.attributes[i];
      if (b.hasAttribute(at.name)) continue;
      v = at.name === 'class' ? keepClasses(a, '') : '';
      if (v) { if (v !== at.value) a.setAttribute('class', v); } else a.removeAttribute(at.name);
    }
  }
  // Makes the live node a look like the fresh node b and keeps a's elements where they match, so the pointer,
  // focus and the hover layer stay. Cards are matched by data-id; frozen (a drag is on) keeps their order.
  function morph(a, b, frozen) {
    if (a.nodeType !== 1) { if (a.nodeValue !== b.nodeValue) a.nodeValue = b.nodeValue; return; }
    var ov = a.classList.contains('tl-ov');
    syncAttrs(a, b);
    // the hover layer is drawn by the view; a node that only now became one (a chart in place of an error well) starts empty
    if (b.classList.contains('tl-ov')) { if (!ov) a.textContent = ''; return; }
    if (a.classList.contains('tl-stack')) { morphCards(a, b, frozen); return; }
    var x = a.firstChild, y = b.firstChild;
    while (y) {
      var ny = y.nextSibling;
      if (!x) a.appendChild(y);
      else if (x.nodeType === y.nodeType && x.nodeName === y.nodeName) { morph(x, y, frozen); x = x.nextSibling; }
      else { var nx = x.nextSibling; a.replaceChild(y, x); x = nx; }
      y = ny;
    }
    while (x) { var rest = x.nextSibling; a.removeChild(x); x = rest; }
  }
  function morphCards(a, b, frozen) {
    var old = {}, at = 0;
    [].slice.call(a.children).forEach(function (c) { var id = c.getAttribute('data-id'); if (id != null) old[id] = c; });
    [].slice.call(b.children).forEach(function (y) {
      var id = y.getAttribute('data-id'), x = id != null ? old[id] : null;
      if (x && x.nodeName === y.nodeName) { morph(x, y, frozen); delete old[id]; } else x = null;
      if (frozen) return;
      var node = x || y, cur = a.children[at];
      if (cur !== node) a.insertBefore(node, cur || null);
      at++;
    });
    if (!frozen) Object.keys(old).forEach(function (id) { if (old[id].parentNode === a) a.removeChild(old[id]); });
  }
  function cssq(s) { return window.CSS && CSS.escape ? CSS.escape(s) : String(s).replace(/"/g, '\\"'); }
  function idxAt(plot, clientX) {
    var r = plot.getBoundingClientRect(), f = (clientX - r.left) / (r.width || 1);
    return Math.max(0, Math.min(n() - 1, Math.floor(f * n())));
  }
  // Bucket indices of a period in the current data; null when none of it is in the window.
  function idxOfMs(ms) {
    var b = bucket(), N = n(), a = Math.floor((ms[0] - WT.fromMs) / b + 1e-6), e = Math.ceil((ms[1] - WT.fromMs) / b - 1e-6) - 1;
    if (e < 0 || a > N - 1 || e < a) return null;
    return [Math.max(0, a), Math.min(N - 1, e)];
  }
  // Something is open over the screen (a dialog, a menu, a list): Esc belongs to it, not to the picked period.
  function overlayOpen() {
    var list = document.querySelectorAll('[role="dialog"], [aria-modal="true"], dialog[open], [role="menu"], [role="listbox"], [aria-expanded="true"]');
    for (var i = 0; i < list.length; i++) if (list[i].getClientRects().length) return true;
    return false;
  }
  function editable(t) { return !!t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName || '')); }
  function scroller(el) {
    for (var p = el.parentElement; p; p = p.parentElement) {
      var s = getComputedStyle(p).overflowY;
      if ((s === 'auto' || s === 'scroll') && p.scrollHeight > p.clientHeight) return p;
    }
    return document.scrollingElement || document.documentElement;
  }
  // The token in hand during a drag: grip, the status mark the card shows (k), the address, probe types.
  function tokenHtml(item, k) {
    return GRIP + '<span class="tl-mk" style="background:var(--' + (MARK[k] || 'ok') + ')"></span><span class="mono">' + esc(L(item.name.ru, item.name.en)) + '</span>' +
      item.tracks.map(function (tr) { var p = probeOf(tr.probe); return '<span class="chip">' + esc(L(p.ru, p.en)) + '</span>'; }).join('');
  }

  /* ---------- the live stack: data updates, hover, picked period, drag, probe switch ---------- */
  function View(el, items, o) {
    this.el = el;
    this.auto = !items;
    this.items = (items || fromTargets()).slice();
    this.o = {};
    for (var k in o || {}) this.o[k] = o[k];
    if (!this.o.layout) this.o.layout = 'row';
    this.o.live = true;
    this.active = {};
    this.rangeMs = this.o.rangeMs ? this.o.rangeMs.slice() : rangeMsOf(this.o.range);
    this.fresh = !!this.rangeMs; // the data already holds rangeStats for the period the screen gave
    this.range = this.drawn = null;
    this.index();
    this.render();
    this.bind();
  }
  View.prototype.index = function () {
    var self = this;
    this.byId = {};
    this.items.forEach(function (it) { self.byId[it.id] = it; });
  };
  View.prototype.co = function (it, tk) {
    var c = {}, k;
    for (k in this.o) c[k] = this.o[k];
    c.range = this.range; c.rangeMs = this.rangeMs; c.core = this.core(); c.unpin = !!this.o.onUnpin;
    if (tk) c.tk = tk;
    if (it) c.active = this.active[it.id] || 0;
    return c;
  };
  View.prototype.html = function () {
    var self = this, tk = ticks();
    var body = '<div class="tl-stack">' + this.items.map(function (it) { return card(it, self.co(it, tk)); }).join('') + '</div>';
    if (!this.o.stack) return body;
    return '<div class="tl-stackwrap" data-layout="' + this.o.layout + '">' + stackHead(this.rangeMs, winKey(this.o.window)) + body +
      '<div class="tl-axisbar">' + xAxisHtml(tk, this.tight) + '</div></div>';
  };
  // Labels with a day in front (yesterday, older dates) may not fit a narrow axis: then a day repeated from the
  // label before is hidden, so the day stays on the first label and where it changes. Measured with full labels.
  // A label that still runs into the one before it (a very narrow axis) is left out.
  View.prototype.fitAxis = function () {
    var ax = this.el.querySelector('.tl-axisbar .tl-xax'), tight = false;
    if (!ax || !ax.clientWidth) return; // hidden: nothing to measure
    var ls = [].slice.call(ax.querySelectorAll('.tl-xl'));
    function clash(off) {
      var prev = null;
      return ls.filter(function (l) {
        if (l.classList.contains('is-off')) return false;
        var r = l.getBoundingClientRect(), c = !!prev && r.left < prev.right + 6;
        if (!c || !off) prev = r;
        return c;
      });
    }
    ls.forEach(function (l) { if (l.classList.contains('is-off')) l.classList.remove('is-off'); });
    if (ax.querySelector('.is-rep')) {
      ax.classList.remove('is-tight');
      tight = clash(false).length > 0;
    }
    this.tight = tight;
    if (ax.classList.contains('is-tight') !== tight) ax.classList.toggle('is-tight', tight);
    if (tight) clash(true).forEach(function (l) { l.classList.add('is-off'); });
  };
  // New data: items from the screen, or from WT.targets when the view was made without them.
  View.prototype.update = function (items, o) {
    var k;
    for (k in o || {}) this.o[k] = o[k];
    if (items) this.items = items.slice();
    else if (this.auto) this.items = this.ordered(fromTargets());
    this.index();
    if (this.rangeMs) this.fresh = true;
    this.render();
  };
  // Right after a drag the core may still send the old order once: the new one holds until the data agrees.
  View.prototype.ordered = function (list) {
    var w = this.want;
    if (!w) return list;
    var ids = list.map(function (it) { return it.id; }).join(',');
    if (ids === w.ids.join(',') || Date.now() > w.until) { this.want = null; return list; }
    var pos = {};
    w.ids.forEach(function (id, i) { pos[id] = i; });
    return list.map(function (it, i) { return { it: it, p: pos[it.id] != null ? pos[it.id] : 1e6 + i }; })
      .sort(function (a, b) { return a.p - b.p; }).map(function (x) { return x.it; });
  };
  // Do the figures in the data belong to the picked period? Live.shown is the request the data answers.
  View.prototype.core = function () {
    var ms = this.rangeMs, sh = window.Live && window.Live.shown;
    if (!ms) return false;
    if (!sh) return this.fresh;
    return !!(sh.range && Math.abs(sh.range[0] - ms[0]) < 1 && Math.abs(sh.range[1] - ms[1]) < 1);
  };
  View.prototype.render = function () {
    // The screen may set view.range itself (bucket indices for the data it is about to show): take it.
    var d = this.drawn, cur = this.range;
    if (!(cur === d || (cur && d && cur[0] === d[0] && cur[1] === d[1]))) { this.rangeMs = rangeMsOf(cur); this.fresh = false; }
    var r = this.rangeMs ? idxOfMs(this.rangeMs) : null, gone = !!this.rangeMs && !r;
    if (gone) { this.rangeMs = null; this.fresh = false; } // the period left the window
    this.range = r;
    this.drawn = r ? r.slice() : null;
    var focus = this.saveFocus(), el = this.el, root = el.firstElementChild, tpl = document.createElement('template');
    tpl.innerHTML = this.html();
    var fresh = tpl.content.firstElementChild;
    if (root && fresh && el.children.length === 1 && root.nodeName === fresh.nodeName &&
        root.classList.contains('tl-stackwrap') === fresh.classList.contains('tl-stackwrap')) {
      morph(root, fresh, !!this.drag);
    } else {
      el.innerHTML = '';
      el.appendChild(tpl.content);
    }
    this.restoreFocus(focus);
    // The screen may render the view into a detached copy and merge it into the page (main.js does):
    // motion and hover go to the page right after that.
    var self = this;
    if (!this.queued) {
      this.queued = true;
      Promise.resolve().then(function () {
        self.queued = false;
        if (!self.el.isConnected) return;
        self.fitAxis();
        self.slide();
        if (self.sel) self.paintSel(); else self.refreshHover();
      });
    }
    if (gone) { if (this.o.onRange) this.o.onRange(null, null); }
  };
  // Live data moved the window on: the charts and the axis slide by that much instead of jumping.
  View.prototype.slide = function () {
    var f = this.frame, from = WT.fromMs, to = WT.toMs, N = n();
    this.frame = { from: from, to: to, n: N };
    if (!f || !WT.live || f.n !== N || Math.abs((f.to - f.from) - (to - from)) > 1) return;
    var dt = from - f.from, span = to - from;
    if (!(dt > 0) || dt > span * 0.1 || (CALM && CALM.matches) || !Element.prototype.animate) return;
    var dx = dt / span * W, pct = dt / span * 100, opt = { duration: 320, easing: 'cubic-bezier(.2,.7,.3,1)' }, sel = !!this.sel;
    this.el.querySelectorAll('.tl svg .mv').forEach(function (p) {
      if (sel && (p.classList.contains('sel') || p.classList.contains('sel-edge'))) return;
      p.animate([{ transform: 'translateX(' + f2(dx) + 'px)' }, { transform: 'translateX(0)' }], opt);
    });
    this.el.querySelectorAll('.tl-xax').forEach(function (x) {
      x.animate([{ transform: 'translateX(' + f2(pct) + '%)' }, { transform: 'translateX(0)' }], opt);
    });
    var self = this;
    clearTimeout(this.slideT);
    this.slideT = setTimeout(function () { if (!self.sel) self.refreshHover(); }, opt.duration + 20);
  };
  View.prototype.saveFocus = function () {
    var a = document.activeElement;
    if (!a || a === document.body || !this.el.contains(a)) return null;
    var c = a.closest('.tl[data-id]'), root = c || this.el, sel = null;
    ['tl-grip', 'tl-unpin', 'tl-status', 'tl-reset'].some(function (k) { if (a.classList.contains(k)) { sel = '.' + k; return true; } });
    if (!sel && a.matches('.tl-seg button')) sel = '.tl-seg button';
    return { el: a, id: c && c.getAttribute('data-id'), sel: sel, i: sel ? [].indexOf.call(root.querySelectorAll(sel), a) : -1 };
  };
  View.prototype.restoreFocus = function (f) {
    if (!f || document.activeElement === f.el) return;
    var t = f.el.isConnected ? f.el : null;
    if (!t && f.sel) {
      var root = f.id != null ? this.el.querySelector('.tl[data-id="' + cssq(f.id) + '"]') : this.el;
      t = root && root.querySelectorAll(f.sel)[Math.max(0, f.i)];
    }
    if (t && t.focus) t.focus({ preventScroll: true });
  };
  View.prototype.plotOf = function (id, k) {
    return this.el.querySelector('.tl[data-id="' + cssq(id) + '"] .tl-plot[data-k="' + k + '"]');
  };
  View.prototype.hover = function (plot, i, rng) {
    var self = this;
    this.el.querySelectorAll('.tl-plot').forEach(function (p) {
      var c = p.closest('.tl[data-id]'), item = c && self.byId[c.getAttribute('data-id')], k = +p.getAttribute('data-k'), ov = p.querySelector('.tl-ov');
      if (!item || !ov) return;
      var h = rng ? '<div class="tl-xh" style="left:' + f2(X(i) / W * 100) + '%"></div>' : hoverLayer(item, k, i, self.co(item), p === plot);
      if (rng && p === plot) h += selLabel(rng[0], rng[1]);
      ov.innerHTML = h;
    });
  };
  View.prototype.clear = function () { this.el.querySelectorAll('.tl-ov').forEach(function (ov) { if (ov.firstChild) ov.innerHTML = ''; }); };
  // The pointer did not move, the data did: show what is under it now.
  View.prototype.refreshHover = function () {
    var h = this.hov, p = h && !this.drag ? this.plotOf(h.id, h.k) : null;
    if (p) this.hover(p, idxAt(p, h.x));
    else if (h) { this.hov = null; this.clear(); }
  };
  View.prototype.paint = function (a, b) {
    this.el.querySelectorAll('.tl-plot svg, .tl-jwell svg').forEach(function (s) {
      var H = +s.getAttribute('viewBox').split(' ')[3], p = s.querySelector('.sel'), e = s.querySelector('.sel-edge');
      if (p) p.setAttribute('d', a == null ? 'M0 0' : rangePath(a, b, H));
      if (e) e.setAttribute('d', a == null ? 'M0 0' : edgePath(a, b, H));
    });
  };
  // A period being dragged out: its start is a moment (it moves with live data), its end is under the pointer.
  View.prototype.selIdx = function () {
    var s = this.sel, p = s && this.plotOf(s.id, s.k);
    if (!p) return null;
    var a = Math.max(0, Math.min(n() - 1, Math.floor((s.from - WT.fromMs) / bucket() + 1e-6))), b = idxAt(p, s.x);
    return [Math.min(a, b), Math.max(a, b), b];
  };
  View.prototype.paintSel = function () {
    var r = this.selIdx();
    if (!r) return;
    this.paint(r[0], r[1]);
    this.hover(this.plotOf(this.sel.id, this.sel.k), r[2], [r[0], r[1]]);
  };
  View.prototype.endSel = function (cancel) {
    var s = this.sel, r = this.selIdx();
    this.sel = null;
    try { if (s.plot.hasPointerCapture && s.plot.hasPointerCapture(s.pid)) s.plot.releasePointerCapture(s.pid); } catch (x) { /* already released */ }
    if (!cancel && r && r[1] - r[0] >= 1) { this.userRange([msAt(r[0]), msAt(r[1] + 1)]); return; }
    this.paint(this.range ? this.range[0] : null, this.range ? this.range[1] : null);
    this.refreshHover();
  };
  // Picked by the user: through setRange (the screen may wrap it, main.js does), then onRange.
  View.prototype.userRange = function (ms) {
    this.setRange(ms ? idxOfMs(ms) : null);
    if (this.o.onRange) this.o.onRange(this.rangeMs ? this.rangeMs.slice() : null, this.range ? this.range.slice() : null);
  };
  View.prototype.setRangeMs = function (ms) {
    var same = ms && this.rangeMs && ms[0] === this.rangeMs[0] && ms[1] === this.rangeMs[1];
    if (!same) { this.rangeMs = ms ? [ms[0], ms[1]] : null; this.fresh = false; }
    this.drawn = this.range; // not an outside change of view.range
    this.render();
  };
  View.prototype.setRange = function (r) { this.setRangeMs(rangeMsOf(r)); };
  View.prototype.indexOf = function (id) {
    for (var i = 0; i < this.items.length; i++) if (this.items[i].id === id) return i;
    return -1;
  };
  // Items in the order of these card ids; the ones not listed keep their place at the end.
  View.prototype.sortItems = function (ids) {
    var pos = {};
    ids.forEach(function (id, i) { pos[id] = i; });
    this.items = this.items.map(function (it, i) { return { it: it, p: pos[it.id] != null ? pos[it.id] : 1e6 + i }; })
      .sort(function (a, b) { return a.p - b.p; }).map(function (x) { return x.it; });
  };
  View.prototype.reordered = function () {
    var ids = this.items.map(function (it) { return it.id; }), rows = [];
    this.items.forEach(function (it) { rows = rows.concat(it.rows || it.tracks.map(function (t) { return t.src && t.src.id; })); });
    this.want = { ids: ids, until: Date.now() + 5000 };
    if (this.o.onOrder) this.o.onOrder(rows.filter(function (x) { return x != null; }), ids);
  };
  View.prototype.bind = function () {
    var self = this, el = this.el;
    el.addEventListener('pointermove', function (e) {
      if (self.drag) return;
      if (self.sel) { self.sel.x = e.clientX; if (self.hov) self.hov.x = e.clientX; self.paintSel(); return; }
      var plot = e.target.closest ? e.target.closest('.tl-plot') : null, c = plot && plot.closest('.tl[data-id]');
      if (!c || !el.contains(plot)) { if (self.hov) { self.hov = null; self.clear(); } return; }
      self.hov = { id: c.getAttribute('data-id'), k: +plot.getAttribute('data-k'), x: e.clientX };
      self.hover(plot, idxAt(plot, e.clientX));
    });
    el.addEventListener('pointerleave', function () { if (!self.sel) { self.hov = null; self.clear(); } });
    el.addEventListener('pointerdown', function (e) {
      if (e.button !== 0 || e.defaultPrevented || self.drag) return; // the screen may run the drag itself
      var g = e.target.closest('.tl-grip');
      if (g) { if (self.o.reorder) self.startDrag(g, e); return; }
      var plot = e.target.closest('.tl-plot'), c = plot && plot.closest('.tl[data-id]');
      if (!c || !self.o.select || !el.contains(plot)) return;
      e.preventDefault();
      self.sel = { id: c.getAttribute('data-id'), k: +plot.getAttribute('data-k'), from: msAt(idxAt(plot, e.clientX)), x: e.clientX, pid: e.pointerId, plot: plot };
      self.hov = { id: self.sel.id, k: self.sel.k, x: e.clientX };
      try { plot.setPointerCapture(e.pointerId); } catch (x) { /* no capture: the pointerup on the window still ends it */ }
      self.paintSel();
    });
    this.onUp = function (e) { if (self.sel) self.endSel(e.type === 'pointercancel'); };
    el.addEventListener('pointercancel', this.onUp);
    window.addEventListener('pointerup', this.onUp);
    // A new width (the window, the settings card) may make the time axis tight or loose again.
    if (window.ResizeObserver) { this.ro = new ResizeObserver(function () { if (el.isConnected) self.fitAxis(); }); this.ro.observe(el); }
    el.addEventListener('click', function (e) {
      var b;
      if (e.target.closest('.tl-reset')) { self.userRange(null); return; }
      if ((b = e.target.closest('.tl-unpin'))) {
        var pc = b.closest('.tl[data-id]');
        if (self.o.onUnpin) self.o.onUnpin(b.getAttribute('data-row'), pc && pc.getAttribute('data-id'));
        return;
      }
      if ((b = e.target.closest('.tl-seg button'))) {
        var sc = b.closest('.tl[data-id]');
        self.active[sc.getAttribute('data-id')] = +b.getAttribute('data-k');
        self.render();
      }
    });
    // Alt+arrow on the grip moves the card by one place.
    el.addEventListener('keydown', function (e) {
      if (!self.o.reorder || !e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') || e.defaultPrevented) return;
      var g = e.target.closest && e.target.closest('.tl-grip'), c = g && g.closest('.tl[data-id]');
      if (!c) return;
      e.preventDefault();
      var i = self.indexOf(c.getAttribute('data-id')), j = i + (e.key === 'ArrowUp' ? -1 : 1);
      if (i < 0 || j < 0 || j >= self.items.length) return;
      self.items.splice(j, 0, self.items.splice(i, 1)[0]);
      self.render();
      self.reordered();
    });
    // Esc during a drag or while a period is dragged out cancels it and goes no further: on the window in the
    // capture phase, before the header (its settings card stays open), as in the summary.
    this.onEsc = function (e) {
      if (e.key !== 'Escape' || !(self.drag || self.sel)) return;
      e.preventDefault();
      e.stopPropagation();
      if (self.drag) self.dropDrag(true); else self.endSel(true);
    };
    window.addEventListener('keydown', this.onEsc, true);
    // Then Esc drops the picked period, unless a dialog or a menu is open (it is theirs).
    this.onKey = function (e) {
      if (!el.isConnected) { self.destroy(); return; }
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      if (!self.rangeMs || overlayOpen() || editable(e.target)) return;
      e.preventDefault();
      self.userRange(null);
    };
    document.addEventListener('keydown', this.onKey);
  };
  View.prototype.destroy = function () {
    document.removeEventListener('keydown', this.onKey);
    window.removeEventListener('keydown', this.onEsc, true);
    window.removeEventListener('pointerup', this.onUp);
    if (this.ro) this.ro.disconnect();
    if (this.drag) this.dropDrag(true);
  };

  /* ---------- drag to reorder: the card fades in place, a line shows the drop place, a short token is in hand ---------- */
  View.prototype.cards = function () { return [].slice.call(this.el.querySelectorAll('.tl-stack > .tl[data-id]')); };
  View.prototype.startDrag = function (g, e) {
    var self = this, cardEl = g.closest('.tl[data-id]');
    if (!cardEl) return;
    e.preventDefault();
    try { g.setPointerCapture(e.pointerId); } catch (x) { /* the document listens anyway */ }
    this.hov = null;
    this.clear();
    var id = cardEl.getAttribute('data-id'), ins = document.createElement('div'), tok = document.createElement('div');
    ins.className = 'tl-ins'; ins.hidden = true;
    var it = this.byId[id], t0 = it.tracks[0], st0 = t0 && !errRow(t0) ? statsOf(t0.src, this.range, this.core()) : null;
    tok.className = 'tl-token'; tok.setAttribute('aria-hidden', 'true'); tok.innerHTML = tokenHtml(it, t0 ? keyOf(t0, st0, !!this.range) : 'ok');
    document.body.appendChild(ins);
    document.body.appendChild(tok);
    cardEl.classList.add('is-source');
    document.documentElement.classList.add('tl-dragging');
    var d = this.drag = { id: id, card: cardEl, to: -1, ins: ins, tok: tok, x: e.clientX, y: e.clientY, sc: scroller(this.el) };
    function move(ev) { d.x = ev.clientX; d.y = ev.clientY; self.dragMove(); }
    function up() { self.dropDrag(false); }
    function cancel() { self.dropDrag(true); }
    d.off = function () {
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerup', up);
      document.removeEventListener('pointercancel', cancel);
    };
    document.addEventListener('pointermove', move);
    document.addEventListener('pointerup', up);
    document.addEventListener('pointercancel', cancel);
    this.dragMove();
    (function loop() {
      if (self.drag !== d) return;
      var whole = d.sc === document.scrollingElement || d.sc === document.documentElement;
      var r = whole ? { top: 0, bottom: window.innerHeight } : d.sc.getBoundingClientRect(), dy = 0;
      if (d.y < r.top + 40) dy = -12; else if (d.y > r.bottom - 64) dy = 12;
      if (dy) { d.sc.scrollTop += dy; self.dragPlace(); }
      requestAnimationFrame(loop);
    })();
  };
  View.prototype.dragMove = function () {
    var d = this.drag;
    d.tok.style.left = Math.round(d.x - 14) + 'px';
    d.tok.style.top = Math.round(d.y - 16) + 'px';
    this.dragPlace();
  };
  View.prototype.dragPlace = function () {
    var d = this.drag, cs = this.cards(), to = cs.length, i, r;
    for (i = 0; i < cs.length; i++) { r = cs[i].getBoundingClientRect(); if (d.y < r.top + r.height / 2) { to = i; break; } }
    d.to = to;
    var from = cs.indexOf(d.card), same = from < 0 || to === from || to === from + 1;
    d.ins.hidden = same;
    if (same) return;
    var lr = cs[0].parentNode.getBoundingClientRect(), y;
    if (to === 0) y = cs[0].getBoundingClientRect().top - 1;
    else if (to === cs.length) y = cs[cs.length - 1].getBoundingClientRect().bottom + 1;
    else y = (cs[to - 1].getBoundingClientRect().bottom + cs[to].getBoundingClientRect().top) / 2;
    d.ins.style.left = Math.round(lr.left) + 'px';
    d.ins.style.width = Math.round(lr.width) + 'px';
    d.ins.style.top = Math.round(y) + 'px';
  };
  View.prototype.dropDrag = function (cancel) {
    var d = this.drag;
    if (!d) return;
    d.off();
    var cs = this.cards(), from = cs.indexOf(d.card), moved = !cancel && d.to >= 0 && !d.ins.hidden && from >= 0;
    this.drag = null;
    d.card.classList.remove('is-source');
    d.ins.remove();
    d.tok.remove();
    document.documentElement.classList.remove('tl-dragging');
    if (moved) {
      var ids = cs.map(function (c) { return c.getAttribute('data-id'); });
      ids.splice(d.to > from ? d.to - 1 : d.to, 0, ids.splice(from, 1)[0]);
      this.sortItems(ids);
    }
    this.render(); // also brings in what a live update kept waiting during the drag
    if (moved) this.reordered();
  };

  /* ---------- the status explanation: only in a tooltip ---------- */
  var hintEl = null;
  function showHint(t) {
    hideHint();
    var r = t.getBoundingClientRect();
    hintEl = document.createElement('div');
    hintEl.className = 'tl-tip tl-hint is-float';
    hintEl.textContent = t.getAttribute('data-hint');
    document.body.appendChild(hintEl);
    var left = Math.min(r.left - 2, document.documentElement.clientWidth - hintEl.offsetWidth - 8);
    hintEl.style.left = (left + window.scrollX) + 'px';
    hintEl.style.top = (r.bottom + window.scrollY + 8) + 'px';
  }
  function hideHint() { if (hintEl) { hintEl.remove(); hintEl = null; } }
  function hintTarget(e) { var t = e.target && e.target.closest ? e.target.closest('[data-hint]') : null; return t && !t.querySelector('.tl-hint') ? t : null; }
  // A hint opened by the pointer stays while the focus moves elsewhere; one opened by the focus goes with it.
  var hintFor = null, hintByFocus = false;
  function openHint(t, byFocus) { showHint(t); hintFor = t; hintByFocus = byFocus; }
  document.addEventListener('pointerover', function (e) { var t = hintTarget(e); if (t) openHint(t, false); });
  document.addEventListener('pointerout', function (e) { var t = hintTarget(e); if (t && !(e.relatedTarget && t.contains(e.relatedTarget))) hideHint(); });
  document.addEventListener('focusin', function (e) { var t = hintTarget(e); if (t) openHint(t, true); });
  document.addEventListener('focusout', function () {
    if (hintEl && (hintByFocus || !(hintFor && hintFor.isConnected && hintFor.matches(':hover')))) hideHint();
  });

  /* ---------- legend: the same swatches are in the header legend ---------- */
  function legendHtml() {
    var hatch = '', th = thresholds();
    for (var x = -16; x < 34; x += 6) hatch += 'M' + x + ' 16L' + (x + 16) + ' 0';
    var sw = function (inner, cls) { return '<span class="lg-sw' + (cls ? ' ' + cls : '') + '"><svg width="32" height="16" viewBox="0 0 32 16" aria-hidden="true">' + inner + '</svg></span>'; };
    var rows = [
      [sw('<path class="lg-line" d="M2 11L8 9L14 12L20 5L26 8L30 7"/>'), 'Задержка', 'Latency'],
      [sw('', 'lg-warn'), th.warn + '-' + th.bad + ' мс', th.warn + '-' + th.bad + ' ms'],
      [sw('', 'lg-bad'), 'выше ' + th.bad + ' мс', 'over ' + th.bad + ' ms'],
      [sw('<rect class="lg-loss" x="13" y="0" width="6" height="16"/>'), 'Потеря', 'Loss'],
      [sw('<rect class="lg-part" x="5" y="11" width="3" height="5"/><rect class="lg-part" x="14" y="11" width="3" height="5"/><rect class="lg-part" x="23" y="11" width="3" height="5"/>'), 'Ответил не на все пробы', 'Answered some probes'],
      [sw('<path class="lg-hatch" d="' + hatch + '"/>'), 'Не отвечает на пробы этого типа', 'No reply to this probe type'],
      [sw('<path class="lg-jit" d="M2 10L7 12L12 6L17 11L22 9L27 12L30 8"/>'), 'Джиттер', 'Jitter'],
      [sw('<rect class="lg-sel" x="8" y="0" width="16" height="16"/><path class="lg-edge" d="M8 0V16M24 0V16"/>'), 'Выделенный период', 'Selected period'],
    ];
    return '<div class="lg">' + rows.map(function (r) { return '<span class="lg-item">' + r[0] + S(r[1], r[2]) + '</span>'; }).join('') + '</div>';
  }

  /* ---------- cards from the data ---------- */
  // Rows of one target (one per probe type) are the tracks of one card; a pinned hop is a card of its own.
  // Card id is the id of its first row. The label goes as in the summary: the address first, below it the core's
  // t.name (for a target by IP its reverse DNS name, for a target by name its IP; for a hop the hop's name).
  function fromTargets(rows) {
    rows = rows || WT.targets || [];
    var byId = {}, groups = {}, out = [];
    rows.forEach(function (t) { byId[t.id] = t; });
    rows.forEach(function (t) {
      var tr = { id: t.id, probe: t.probe, status: t.status, src: t };
      if (t.hopOf) {
        var of = byId[t.hopOf];
        out.push({ id: t.id, rows: [t.id], pinned: true, hop: t.hop, of: of ? of.host : '', name: nm(t.host), addr: t.name ? nm(t.name) : null, addrMono: true, tracks: [tr] });
        return;
      }
      var key = t.targetId != null ? 't' + t.targetId : 'h' + t.host, it = groups[key];
      if (it) { it.rows.push(t.id); it.tracks.push(tr); return; }
      var sub = t.name && t.name !== t.host ? t.name : null;
      it = groups[key] = { id: t.id, rows: [t.id], name: nm(t.host), addr: sub ? nm(sub) : null, addrMono: true, tracks: [tr] };
      out.push(it);
    });
    return out;
  }
  // Single cards by row id (a target row alone, a pinned hop with its target's address), for checks and the part's page.
  function items() {
    var rows = WT.targets || [], out = { all: fromTargets(rows) };
    rows.forEach(function (t) {
      var list = t.hopOf ? rows.filter(function (x) { return x.id === t.hopOf; }).concat([t]) : [t], its = fromTargets(list);
      out[t.id] = its[its.length - 1];
    });
    return out;
  }

  window.WTT = {
    EN: EN, S: S, L: L, card: card, stackHtml: stackHtml, dragHtml: dragHtml, legendHtml: legendHtml, items: items,
    fromTargets: fromTargets,
    view: function (el, list, o) { return new View(el, list, o); },
    tAt: tAt, tEnd: tEnd, timeText: timeText, spanText: spanText, statsOf: statsOf,
  };
})();
