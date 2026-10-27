// Whilo Trace, part «history and report»: the day strip with the window frame, the «viewing the past» bar,
// the first-run screen, texts for error codes and the «Report for ISP» dialog with its saved result.
// Data comes from window.WT (docs/ui-contract.md), commands go through window.API, the screen gets the user's
// choices through callbacks (HE.history onEnd, HE.pastBar onNow, HE.empty onPick, HE.report onClose).
// HE.errorInfo is the one place for the texts of error codes: every part takes them from here.
// Every visible text is written in the interface language and carries data-en.
(function () {
  'use strict';
  var WT = window.WT;
  var DAY = 86400000, SLOTS = 200, MIN = 60000, NB = ' ';

  /* ---------- texts and numbers ---------- */
  function isEN() { return document.documentElement.dataset.lang === 'en'; }
  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  function pick(ru, en) { return isEN() ? en : ru; }
  // Visible text in the interface language with data-en (leaf elements only: theme.js rewrites their text).
  function L(ru, en, cls, tag) {
    tag = tag || 'span';
    return '<' + tag + (cls ? ' class="' + cls + '"' : '') + ' data-en="' + esc(en) + '">' + esc(isEN() ? en : ru) + '</' + tag + '>';
  }
  function setText(el, ru, en) {
    if (!el) return;
    var t = isEN() ? en : ru;
    if (el.textContent !== t) el.textContent = t;
    if (el.getAttribute('data-en') !== en) el.setAttribute('data-en', en);
  }
  function n1(v, en) { var s = WT.fmt(v); return en ? s.replace(',', '.') : s; }
  function pc(v, en) { return (v ? n1(v, en) : '0') + (en ? '%' : NB + '%'); }
  function msTxt(v, en) { return n1(v, en) + (en ? ' ms' : NB + 'мс'); }
  function nInt(v, en) { return String(Math.round(v)).replace(/\B(?=(\d{3})+(?!\d))/g, en ? ',' : NB); }
  function ruN(n, one, few, many) {
    var a = n % 10, b = n % 100;
    if (a === 1 && b !== 11) return one;
    if (a >= 2 && a <= 4 && (b < 12 || b > 14)) return few;
    return many;
  }
  // [ru, en]
  function cnt(n, ru3, en2) { return [nInt(n, false) + ' ' + ruN(n, ru3[0], ru3[1], ru3[2]), nInt(n, true) + ' ' + (n === 1 ? en2[0] : en2[1])]; }
  function andList(a, en) { return a.length < 2 ? a.join('') : a.slice(0, -1).join(', ') + (en ? ' and ' : ' и ') + a[a.length - 1]; }
  function cap(s) { return s.charAt(0).toUpperCase() + s.slice(1); }
  function clamp(v, a, b) { return Math.max(a, Math.min(b, v)); }
  // Seconds or ms as the header writes them: «2,5», «1», «0,5» (en «2.5»), one decimal at most.
  function num1(v, en) { var s = String(Math.round(v * 10) / 10); return en ? s : s.replace('.', ','); }
  // «каждые 2,5 с» / «every 2.5 s»; one second reads as words.
  function every(v) {
    if (Math.abs(v - 1) < 1e-9) return ['каждую секунду', 'every second'];
    return ['каждые ' + num1(v, false) + NB + 'с', 'every ' + num1(v, true) + ' s'];
  }

  /* ---------- time ---------- */
  // The strip and the past bar use the system time zone; the report uses the zone picked in the dialog
  // (tz 'utc', otherwise the system's). Days are counted from the snapshot's «now».
  function nowMs() { return WT.nowMs || Date.now(); }
  function offMin(ms, tz) { return tz === 'utc' ? 0 : -new Date(ms).getTimezoneOffset(); }
  function fields(ms, tz) {
    var sh = ms + offMin(ms, tz) * MIN, d = new Date(sh);
    return { y: d.getUTCFullYear(), mo: d.getUTCMonth() + 1, d: d.getUTCDate(), h: d.getUTCHours(), mi: d.getUTCMinutes(), s: d.getUTCSeconds(), day: Math.floor(sh / DAY) };
  }
  function pad(n) { return (n < 10 ? '0' : '') + n; }
  function hm(ms, tz) { var f = fields(ms, tz); return pad(f.h) + ':' + pad(f.mi); }
  function hms(ms, tz) { var f = fields(ms, tz); return pad(f.h) + ':' + pad(f.mi) + ':' + pad(f.s); }
  // 27.10.2026 in Russian; ISO 2026-10-27 in English with non-breaking hyphens, so the date never wraps.
  function dateOf(ms, tz, en) { var f = fields(ms, tz); return en ? f.y + '‑' + pad(f.mo) + '‑' + pad(f.d) : pad(f.d) + '.' + pad(f.mo) + '.' + f.y; }
  // The day relative to now in the zone tz (as in fields): «сегодня», «вчера», otherwise the date.
  function dayOf(ms, en, tz) {
    var k = fields(ms, tz).day - fields(nowMs(), tz).day;
    if (k === 0) return en ? 'today' : 'сегодня';
    if (k === -1) return en ? 'yesterday' : 'вчера';
    return dateOf(ms, tz, en);
  }
  // «08:50-09:00»; with the day «сегодня 08:50-09:00»; over midnight «вчера 23:55 - сегодня 00:05».
  function span(a, b, withDay, en) {
    if (fields(a).day === fields(b).day) return (withDay ? dayOf(a, en) + ' ' : '') + hm(a) + '-' + hm(b);
    return dayOf(a, en) + ' ' + hm(a) + ' - ' + dayOf(b, en) + ' ' + hm(b);
  }
  // A report period: date and time; seconds only when a border is not on a whole minute.
  function rangeText(a, b, tz, en) {
    var A = fields(a, tz), B = fields(b, tz), sec = A.s || B.s, ta = sec ? hms(a, tz) : hm(a, tz), tb = sec ? hms(b, tz) : hm(b, tz);
    if (A.day === B.day) return dateOf(a, tz, en) + ', ' + ta + '-' + tb;
    return dateOf(a, tz, en) + ' ' + ta + ' - ' + dateOf(b, tz, en) + ' ' + tb;
  }
  // «10 мин», «1 ч», «1 ч 30 мин», «2 мин 55 с».
  function dur(len, en) {
    var s = Math.round(len / 1000), h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60), x = s % 60, out = [], sp = en ? ' ' : NB;
    if (h) out.push(h + sp + (en ? 'h' : 'ч'));
    if (m) out.push(m + sp + (en ? 'min' : 'мин'));
    if (x || !out.length) out.push(x + sp + (en ? 's' : 'с'));
    return out.join(' ');
  }
  function winOf(len) { var k = null; WT.windows.forEach(function (w) { if (Math.abs(WT.windowSeconds[w[0]] * 1000 - len) < 1000) k = w[0]; }); return k; }
  function winName(len, en) {
    var k = winOf(len), w = k && WT.windows.filter(function (x) { return x[0] === k; })[0];
    return w ? (en ? w[2] : w[1]) : dur(len, en);
  }
  // The shortest window that holds a period of len ms.
  function winFor(len) {
    for (var i = 0; i < WT.windows.length; i++) if (WT.windowSeconds[WT.windows[i][0]] * 1000 >= len - 1000) return WT.windows[i][0];
    return WT.windows[WT.windows.length - 1][0];
  }
  function stamp(ms, tz) { var f = fields(ms, tz); return f.y + pad(f.mo) + pad(f.d) + '-' + pad(f.h) + pad(f.mi) + pad(f.s); }
  function offText(min) { var a = Math.abs(min); return 'UTC' + (min < 0 ? '-' : '+') + pad(Math.floor(a / 60)) + ':' + pad(a % 60); }
  // Labels on a day strip: «HH:MM», with the day in front when it is not the strip's last day
  // (and on the last label too when the strip ends before today). tz: as in fields (the report's zone).
  function tickLabels(times, tz) {
    var endDay = fields(times[times.length - 1], tz).day, today = fields(nowMs(), tz).day;
    return times.map(function (t, k) {
      var pre = fields(t, tz).day !== endDay || (k === times.length - 1 && endDay !== today);
      return [(pre ? dayOf(t, false, tz) + ' ' : '') + hm(t, tz), (pre ? dayOf(t, true, tz) + ' ' : '') + hm(t, tz)];
    });
  }

  /* ---------- rows ---------- */
  function tg(id) { return WT.targets.filter(function (x) { return x.id === id; })[0]; }
  var NAME_EN = { 'роутер': 'router' };
  function nm(t, en) { var n = t.name || t.host; return en ? (NAME_EN[n] || n) : n; }
  function probeLabel(t) { var p = WT.probes[t.probe]; return p ? p.label : String(t.probe || '').toUpperCase(); }
  function chip(t) { var l = probeLabel(t); return '<span class="chip" data-en="' + esc(l) + '">' + esc(l) + '</span>'; }
  function mono(s, cls) { return '<span class="mono' + (cls ? ' ' + cls : '') + '" data-en="' + esc(s) + '">' + esc(s) + '</span>'; }
  function runs(list) {
    var out = [], cur = null;
    list.forEach(function (i) { if (cur && i === cur[1] + 1) cur[1] = i; else { cur = [i, i]; out.push(cur); } });
    return out;
  }
  function byNum(a, b) { return a - b; }
  function toSet(list) { var s = {}; (list || []).forEach(function (i) { s[i] = true; }); return s; }

  /* ---------- icons: one line, stroke currentColor ---------- */
  function svg(vb, d) { return '<svg class="ico" viewBox="' + vb + '" aria-hidden="true">' + d + '</svg>'; }
  var I = {
    toNow: svg('0 0 16 16', '<path d="M2.5 8h8.5M8 5l3 3-3 3M13.5 3.5v9"/>'),
    prev: svg('0 0 16 16', '<path d="M9.5 3.5L5 8l4.5 4.5"/>'),
    next: svg('0 0 16 16', '<path d="M6.5 3.5L11 8l-4.5 4.5"/>'),
    past: svg('0 0 20 20', '<path d="M4.3 10a5.7 5.7 0 1 0 1.7-4.05"/><path d="M3.6 3.3v3.3h3.3"/><path d="M10 6.9V10l2.3 1.5"/>'),
    close: svg('0 0 16 16', '<path d="M4 4l8 8M12 4l-8 8"/>'),
    check: svg('0 0 16 16', '<path d="M3.5 8.5l3 3 6-7"/>'),
    copy: svg('0 0 16 16', '<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 3.5V3a.5.5 0 0 0-.5-.5H3.5a1 1 0 0 0-1 1V10a.5.5 0 0 0 .5.5h.5"/>'),
    folder: svg('0 0 16 16', '<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.7l1.5 1.6h4.8A1.5 1.5 0 0 1 14 6.1v5.4a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z"/>'),
    chev: svg('0 0 16 16', '<path d="M4.5 6.5L8 10l3.5-3.5"/>'),
    nohost: svg('0 0 20 20', '<circle cx="8.5" cy="8.5" r="5.5"/><path d="M12.5 12.5L17 17"/><path d="M6.8 6.8l3.4 3.4M10.2 6.8l-3.4 3.4"/>'),
    globe: svg('0 0 20 20', '<circle cx="10" cy="10" r="7"/><path d="M3 10h14M10 3c1.9 2.1 2.9 4.4 2.9 7s-1 4.9-2.9 7c-1.9-2.1-2.9-4.4-2.9-7s1-4.9 2.9-7z"/>'),
    lock: svg('0 0 20 20', '<rect x="4.5" y="9" width="11" height="8" rx="2"/><path d="M7 9V6.5a3 3 0 0 1 6 0V9"/>'),
    route: svg('0 0 40 40', '<circle cx="8" cy="29" r="3"/><circle cx="20" cy="14" r="3"/><circle cx="32" cy="21" r="3"/><path d="M9.9 26.6l8.2-10.2M22.6 15.5l6.8 4"/>'),
    plus: svg('0 0 16 16', '<path d="M8 3.5v9M3.5 8h9"/>'),
    noreply: svg('0 0 16 16', '<circle cx="8" cy="8" r="5.5"/><path d="M4.1 11.9l7.8-7.8"/>'),
  };

  /* ---------- loss marks of the day strip ---------- */
  // The core sends 24 hours of loss marks that end where the window ends (WT.history). The strip can show
  // another day than the last snapshot, so every piece received is kept by time and the strip takes the piece
  // that covers it best. Loss in the window on screen comes from the rows' buckets, which are finer.
  var PIECES = [];
  function addPiece(h) {
    if (!h || !(h.toMs > h.fromMs)) return;
    var slot = (h.toMs - h.fromMs) / (h.slots || SLOTS), key = h.fromMs + ':' + h.toMs + ':' + (h.loss || []).join(',');
    for (var i = 0; i < PIECES.length; i++) if (PIECES[i].key === key) { PIECES.push(PIECES.splice(i, 1)[0]); return; }
    PIECES.push({ key: key, from: h.fromMs, to: h.toMs, marks: runs((h.loss || []).slice().sort(byNum)).map(function (r) {
      return [h.fromMs + r[0] * slot, h.fromMs + (r[1] + 1) * slot];
    }) });
    if (PIECES.length > 12) PIECES.shift();
  }
  function windowMarks(s) {
    if (!s || !s.targets || !(s.bucketMs > 0)) return [];
    var lost = {};
    s.targets.forEach(function (t) { (t.loss || []).forEach(function (i) { lost[i] = 1; }); });
    return runs(Object.keys(lost).map(Number).sort(byNum)).map(function (r) { return [s.fromMs + r[0] * s.bucketMs, s.fromMs + (r[1] + 1) * s.bucketMs]; });
  }
  // Marks inside [a, b], merged; covered: the best piece covers the whole strip (up to two slots).
  function marksFor(a, b, extra) {
    var best = null, bo = 0, list = [], out = [];
    for (var i = PIECES.length - 1; i >= 0; i--) {
      var p = PIECES[i], ov = Math.min(b, p.to) - Math.max(a, p.from);
      if (ov > bo + 1000) { best = p; bo = ov; }
    }
    if (best) list = best.marks.slice();
    list = list.concat(extra || []).map(function (m) { return [Math.max(a, m[0]), Math.min(b, m[1])]; })
      .filter(function (m) { return m[1] > m[0]; }).sort(function (x, y) { return x[0] - y[0]; });
    list.forEach(function (m) {
      var last = out[out.length - 1];
      if (last && m[0] <= last[1]) last[1] = Math.max(last[1], m[1]); else out.push(m.slice());
    });
    return { marks: out, covered: bo >= (b - a) - 2 * DAY / SLOTS };
  }
  // History of a day the strip shows but no snapshot covered yet: a small snapshot just for its history.
  var FETCHING = {};
  function fetchDay(end, live, done) {
    var k = live ? 'live' : String(Math.round(end / MIN));
    if (FETCHING[k] || !window.API) return;
    FETCHING[k] = true;
    var free = function () { setTimeout(function () { delete FETCHING[k]; }, live ? 60000 : 10000); };
    window.API.call('snapshot', { req: { window: WT.windows[0][0], endMs: live ? null : Math.round(end), range: null, trace: null, buckets: 10 } })
      .then(function (s) { addPiece(s && s.history); free(); if (done) done(); }, free);
  }
  function pctOf(len) { return (len / DAY * 100) + '%'; }
  function markHtml(list, a) {
    return list.map(function (m) { return '<i class="hx-mark" style="left:' + pctOf(m[0] - a) + ';width:' + pctOf(m[1] - m[0]) + '"></i>'; }).join('');
  }
  var GRID = [20, 40, 60, 80].map(function (p) { return '<i class="hx-grid" style="left:' + p + '%"></i>'; }).join('');
  function fixNarrow(root) {
    var fr = root.querySelectorAll('.hx-frame[data-w]');
    for (var i = 0; i < fr.length; i++) fr[i].classList.toggle('narrow', fr[i].parentElement.clientWidth * Number(fr[i].dataset.w) / DAY < 14);
  }

  /* ---------- day strip ---------- */
  var STRIPS = [], LOCAL = null;
  function livePage() { var n = nowMs(); return { a: n - DAY, b: n, live: true }; }
  function stripHtml(empty) {
    var pg = livePage(), tk = tickLabels([0, 1, 2, 3, 4, 5].map(function (k) { return pg.a + DAY * k / 5; }));
    var back = pick('На сутки раньше', 'One day back'), fwd = pick('На сутки позже', 'One day forward');
    var nav = '<div class="hx-nav">' +
      '<button type="button" class="btn btn-sm btn-icon hx-prev" title="' + esc(back) + '" aria-label="' + esc(back) + '"' + (empty ? ' disabled' : '') + '>' + I.prev + '</button>' +
      '<button type="button" class="btn btn-sm btn-icon hx-next" disabled title="' + esc(fwd) + '" aria-label="' + esc(fwd) + '">' + I.next + '</button></div>';
    var lane = empty
      ? '<div class="hx-lane"><div class="hx-clip"></div>' + L('Отметки потерь появятся после первых проб', 'Loss marks appear after the first probes', 'hx-empty') + '</div>'
      : '<div class="hx-lane" role="slider" tabindex="0" aria-label="' + esc(pick('Рамка окна на полосе истории', 'Window frame on the history strip')) + '">' +
        '<div class="hx-clip">' + GRID + '<i class="hx-sel"></i><span class="hx-marks"></span></div><i class="hx-frame"></i><i class="hx-cursor" hidden></i></div>';
    var ticks = tk.map(function (p, k) {
      return '<span class="hx-tk num' + (k === 0 ? ' first' : k === 5 ? ' last' : '') + '" style="left:' + (k * 20) + '%" data-en="' + esc(p[1]) + '">' + esc(isEN() ? p[1] : p[0]) + '</span>';
    }).join('');
    var state = empty ? '' :
      '<div class="hx-state"><span class="hx-win"></span><span class="hx-range num"></span>' +
      '<span class="hx-live"><i class="dot" aria-hidden="true"></i>' + L('сейчас', 'live') + '</span>' +
      '<button type="button" class="btn btn-sm hx-now">' + I.toNow + L('К текущему моменту', 'Back to now') + '</button></div>';
    return '<section class="hx' + (empty ? ' is-empty' : '') + '" data-mode="live" aria-label="' + esc(pick('История за сутки', 'Last 24 hours')) + '">' +
      '<div class="hx-top"><div class="hx-title">' + L('История за сутки', 'Last 24 hours', 'hx-name') +
      (empty ? L('пока пусто', 'empty so far', 'hx-sub') : L('отметки потерь всех целей · щелкните по полосе или перетащите рамку', 'loss marks of all targets · click the strip or drag the frame', 'hx-sub')) + '</div>' +
      state + '</div>' +
      '<div class="hx-body">' + nav + lane + '<span></span><div class="hx-ticks">' + ticks + '<span class="hx-read" hidden></span></div></div></section>';
  }

  // Readout under (in the app: above) the strip; axis labels under it hide so nothing overlaps.
  function placeReadout(hx, frac, txt, acc) {
    var box = hx.querySelector('.hx-ticks'), rd = box.querySelector('.hx-read'), tks = box.querySelectorAll('.hx-tk');
    for (var i = 0; i < tks.length; i++) tks[i].classList.remove('off');
    if (!txt) { rd.hidden = true; return; }
    setText(rd, txt[0], txt[1]);
    rd.classList.toggle('acc', !!acc);
    rd.hidden = false;
    var W = box.clientWidth, rw = rd.offsetWidth, x = Math.max(0, Math.min(W - rw, frac * W - rw / 2));
    rd.style.left = x + 'px';
    var br = box.getBoundingClientRect(), a = x - 10, b = x + rw + 10;
    for (var k = 0; k < tks.length; k++) {
      var r = tks[k].getBoundingClientRect(), l = r.left - br.left, rr = r.right - br.left;
      if (rr > a && l < b) tks[k].classList.add('off');
    }
  }

  // The day strip in el. o.onEnd(endMs): the user moved the window frame, null is back to now;
  // the screen calls Live.set({ endMs }) (without o.onEnd the strip does it itself).
  // The strip follows WT on every Live.onData; hover, drag and focus survive live updates.
  function history(el, o) {
    if (el._he && el._he.kind === 'history') { if (o) el._he.o = o; el._he.update(); return el._he; }
    var st = { pageEnd: null, hover: null, drag: null, local: null, want: null, keyT: 0, empty: null, pg: null, marks: [] };
    var ctl = { kind: 'history', el: el, st: st, o: o || {}, update: update, render: render, toNow: toNow };
    el._he = ctl;
    STRIPS.push(ctl);

    function hx() { return el.querySelector('.hx'); }
    // A strip whose element left the page for good (the screen built a new one) stops following the data.
    function gone() {
      if (el._he !== ctl) return true;
      if (el.isConnected) { ctl.seen = true; return false; }
      return !!ctl.seen;
    }
    function build() {
      st.empty = !WT.targets.length;
      st.drag = null; st.local = null; st.hover = null;
      el.innerHTML = stripHtml(st.empty);
      if (!st.empty) bind();
    }
    // The window on screen, or where the user holds or sent the frame until the new snapshot arrives.
    function frame() {
      var W = Math.max(1000, WT.toMs - WT.fromMs), b = st.local != null ? st.local : WT.toMs;
      return { W: W, a: b - W, b: b };
    }
    // The day shown: ends now (live page) or at pageEnd; it moves so the frame always stays inside.
    function page(fr) {
      var now = nowMs(), b = st.pageEnd == null ? now : st.pageEnd;
      if (fr.b > b) b = fr.b;
      if (fr.a < b - DAY) b = fr.a + DAY;
      var live = b >= now - 1000;
      return { a: (live ? now : b) - DAY, b: live ? now : b, live: live };
    }
    function isLive(fr, pg) { return st.local != null ? pg.live && fr.b >= pg.b - 1000 : !!WT.live; }
    function satisfied() {
      var w = st.want;
      return !!w && (w.end == null ? !!WT.live : !WT.live && Math.abs(WT.toMs - w.end) < 1500);
    }
    function update() {
      if (gone()) return;
      if (WT.history) addPiece(WT.history);
      if (!hx() || st.empty !== !WT.targets.length) build();
      if (st.want && satisfied()) { st.want = null; if (!st.drag) st.local = null; }
      if (WT.live && st.local == null) st.pageEnd = null;
      render();
    }
    function setTicks(h, pg) {
      var tk = tickLabels([0, 1, 2, 3, 4, 5].map(function (k) { return pg.a + DAY * k / 5; })), list = h.querySelectorAll('.hx-tk');
      for (var i = 0; i < list.length; i++) setText(list[i], tk[i][0], tk[i][1]);
    }
    function render() {
      var h = hx();
      if (!h || gone()) return;
      if (st.empty) { setTicks(h, livePage()); return; }
      var fr = frame(), pg = page(fr), live = isLive(fr, pg), en = isEN();
      st.pageEnd = pg.live ? null : pg.b;
      st.pg = pg;
      h.dataset.mode = live ? 'live' : 'past';
      var lane = h.querySelector('.hx-lane'), sel = h.querySelector('.hx-sel'), fe = h.querySelector('.hx-frame');
      var l = clamp((fr.a - pg.a) / DAY * 100, 0, 100), r = clamp((fr.b - pg.a) / DAY * 100, 0, 100);
      sel.style.left = fe.style.left = l + '%';
      sel.style.width = fe.style.width = (r - l) + '%';
      fe.classList.toggle('narrow', lane.clientWidth * (r - l) / 100 < 14);
      setText(h.querySelector('.hx-win'), 'окно ' + winName(fr.W, false), winName(fr.W, true) + ' window');
      setText(h.querySelector('.hx-range'), span(fr.a, fr.b, !live, false), span(fr.a, fr.b, !live, true));
      lane.setAttribute('aria-valuemin', String(Math.round(fr.W / MIN)));
      lane.setAttribute('aria-valuemax', String(DAY / MIN));
      lane.setAttribute('aria-valuenow', String(Math.round((fr.b - pg.a) / MIN)));
      lane.setAttribute('aria-valuetext', span(fr.a, fr.b, true, en));
      h.querySelector('.hx-prev').disabled = false;
      h.querySelector('.hx-next').disabled = pg.live;
      var mk = marksFor(pg.a, pg.b, windowMarks(WT)), box = h.querySelector('.hx-marks'), html = markHtml(mk.marks, pg.a);
      if (box._html !== html) { box.innerHTML = html; box._html = html; }
      st.marks = mk.marks;
      if (!mk.covered && !st.want && !st.drag) fetchDay(pg.b, pg.live, render);
      setTicks(h, pg);
      var cur = h.querySelector('.hx-cursor');
      if (st.drag) {
        cur.hidden = true;
        placeReadout(h, ((fr.a + fr.b) / 2 - pg.a) / DAY, [span(fr.a, fr.b, true, false), span(fr.a, fr.b, true, true)], true);
      } else if (st.hover != null) {
        var s = DAY / SLOTS, k = clamp(Math.floor(st.hover * SLOTS), 0, SLOTS - 1), t0 = pg.a + k * s, t1 = t0 + s;
        var lost = st.marks.some(function (m) { return m[0] < t1 && m[1] > t0; });
        cur.hidden = false;
        cur.style.left = (st.hover * 100) + '%';
        placeReadout(h, st.hover, [span(t0, t1, true, false) + (lost ? ' · потери' : ' · без потерь'), span(t0, t1, true, true) + (lost ? ' · loss' : ' · no loss')]);
      } else {
        cur.hidden = true;
        placeReadout(h, 0, null);
      }
      LOCAL = st.local != null ? { a: fr.a, b: fr.b, live: live } : null;
      BARS.forEach(function (b) { b.render(); });
    }
    // Send the frame to the screen: null is back to now. The frame stays where it was sent until the data comes.
    function commit(end, pg) {
      pg = pg || page(frame());
      var live = end == null || (pg.live && end >= pg.b - 1000);
      st.local = live ? pg.b : end;
      if (live) st.pageEnd = null;
      var want = st.want = { end: live ? null : Math.round(end) };
      render();
      var f = ctl.o.onEnd || function (x) { return window.Live && window.Live.set({ endMs: x }); }, p = null;
      try { p = f(want.end); } catch (err) { console.error(err); }
      var settle = function () {
        if (st.want !== want) return;
        if (!satisfied() && !want.retried && window.Live && window.Live.refresh) {
          // The snapshot that came back was asked before this change: ask once more.
          want.retried = true;
          window.Live.refresh().then(settle, settle);
          return;
        }
        st.want = null;
        if (!st.drag) st.local = null;
        render();
      };
      if (p && p.then) p.then(settle, settle);
      setTimeout(settle, 4000);
    }
    function day(dir) {
      var fr = frame(), pg = page(fr), now = nowMs(), pend = pg.b + dir * DAY, end = fr.b + dir * DAY;
      if (dir > 0 && pend >= now - 1000) {
        st.pageEnd = null;
        commit(end >= now - 1000 ? null : end, livePage());
        return;
      }
      st.pageEnd = pend;
      commit(end, { a: pend - DAY, b: pend, live: false });
    }
    function toNow() { clearTimeout(st.keyT); st.pageEnd = null; commit(null, livePage()); }
    function bind() {
      var h = hx(), lane = h.querySelector('.hx-lane');
      function at(e, pg) { var r = lane.getBoundingClientRect(); return pg.a + clamp((e.clientX - r.left) / Math.max(1, r.width), 0, 1) * DAY; }
      function tol() { return 6 / Math.max(1, lane.getBoundingClientRect().width) * DAY; }
      function move(e) {
        var d = st.drag, end = clamp(at(e, d.pg) - d.grab + d.W, d.pg.a + d.W, d.pg.b);
        if (end > d.pg.b - tol()) end = d.pg.b;
        st.local = end; st.hover = null;
        render();
      }
      // Esc while dragging cancels the drag before anything else on the page takes the key (as in the summary).
      function escDrag(e) {
        if (e.key !== 'Escape' || !st.drag) return;
        e.preventDefault(); e.stopPropagation();
        stop(false);
      }
      lane.addEventListener('pointerdown', function (e) {
        if (e.button !== 0) return;
        var fr = frame(), pg = page(fr);
        if (fr.W >= DAY - 1000) return;
        var m = at(e, pg);
        clearTimeout(st.keyT);
        st.drag = { grab: m >= fr.a && m <= fr.b ? m - fr.a : fr.W / 2, pg: pg, W: fr.W, from: fr.b };
        try { lane.setPointerCapture(e.pointerId); } catch (err) { /* synthetic event without an active pointer */ }
        h.classList.add('is-drag');
        window.addEventListener('keydown', escDrag, true);
        lane.focus({ preventScroll: true });
        move(e);
      });
      lane.addEventListener('pointermove', function (e) {
        if (st.drag) { move(e); return; }
        var fr = frame(), pg = page(fr), m = at(e, pg), t = tol();
        st.hover = (m - pg.a) / DAY;
        // Over the frame the cursor says «grab»: the frame is dragged; elsewhere a click moves the frame there.
        lane.classList.toggle('on-frame', fr.W < DAY - 1000 && m >= fr.a - t && m <= fr.b + t);
        render();
      });
      function stop(keep) {
        if (!st.drag) return;
        var d = st.drag, end = st.local;
        st.drag = null;
        h.classList.remove('is-drag');
        window.removeEventListener('keydown', escDrag, true);
        if (keep && end != null && Math.abs(end - d.from) > 500) commit(end, d.pg);
        else { st.local = null; render(); }
      }
      lane.addEventListener('pointerup', function () { stop(true); });
      lane.addEventListener('pointercancel', function () { stop(false); });
      lane.addEventListener('pointerleave', function () { if (!st.drag) { st.hover = null; lane.classList.remove('on-frame'); render(); } });
      lane.addEventListener('keydown', function (e) {
        var fr = frame(), pg = page(fr), k = e.key, end;
        if (k === 'ArrowLeft') end = Math.max(pg.a + fr.W, fr.b - fr.W);
        else if (k === 'ArrowRight') end = Math.min(pg.b, fr.b + fr.W);
        else if (k === 'Home') end = pg.a + fr.W;
        else if (k === 'End') end = pg.b;
        else return;
        e.preventDefault();
        if (st.drag) return;
        st.hover = null; st.local = end;
        render();
        // Keys move the frame at once; the screen gets the last position only.
        clearTimeout(st.keyT);
        st.keyT = setTimeout(function () { commit(st.local, pg); }, 300);
      });
      h.querySelector('.hx-prev').addEventListener('click', function () { day(-1); });
      h.querySelector('.hx-next').addEventListener('click', function () { day(1); });
      h.querySelector('.hx-now').addEventListener('click', function (e) {
        var had = document.activeElement === e.currentTarget;
        toNow();
        // The button hides with the past: the focus goes to the strip, not to the page.
        if (had) lane.focus({ preventScroll: true });
      });
    }

    build();
    update();
    if (window.Live && window.Live.onData) window.Live.onData(function () { update(); });
    return ctl;
  }
  function stripOn() { for (var i = 0; i < STRIPS.length; i++) if (STRIPS[i].el.isConnected && STRIPS[i].el._he === STRIPS[i]) return STRIPS[i]; return null; }

  /* ---------- «viewing the past» bar (5A) ---------- */
  var BARS = [];
  // The bar in el (shown only in the past). o.onNow(): «Back to now»; without it the strip goes back itself.
  function pastBar(el, o) {
    if (el._he && el._he.kind === 'bar') { if (o) el._he.o = o; el._he.render(); return el._he; }
    var ctl = { kind: 'bar', el: el, o: o || {}, render: render, update: render };
    el._he = ctl;
    BARS.push(ctl);
    function render() {
      if (el._he !== ctl || (!el.isConnected && ctl.seen)) return;
      if (el.isConnected) ctl.seen = true;
      var fr = LOCAL || (WT.live ? null : { a: WT.fromMs, b: WT.toMs, live: false }), past = !!fr && !fr.live && WT.targets.length > 0;
      if (el.hidden === past) el.hidden = !past;
      if (!past) return;
      if (!el.querySelector('.pb')) {
        el.innerHTML = '<div class="pb" role="status">' + I.past + '<div class="pb-text"><span class="pb-title"></span>' +
          L('Сводка и таймлайны показывают этот период. Опрос идет, новые данные сохраняются.', 'The summary and timelines show this period. Probing goes on, new data is saved.', 'pb-sub') +
          '</div><button type="button" class="btn btn-primary pb-now">' + I.toNow + L('К текущему моменту', 'Back to now') + '</button></div>';
      }
      setText(el.querySelector('.pb-title'), 'Смотрю прошлое: ' + span(fr.a, fr.b, true, false), 'Viewing the past: ' + span(fr.a, fr.b, true, true));
    }
    el.addEventListener('click', function (e) {
      var b = e.target.closest('.pb-now');
      if (!b) return;
      var s = stripOn(), had = document.activeElement === b;
      if (ctl.o.onNow) { if (s) s.st.pageEnd = null; ctl.o.onNow(); }
      else if (s) s.toNow();
      else if (window.Live) window.Live.set({ endMs: null });
      // The bar hides with the past: the focus goes to the strip, not to the page.
      var lane = had && s && s.el.querySelector('.hx-lane[tabindex]');
      if (lane) lane.focus({ preventScroll: true });
    });
    render();
    if (window.Live && window.Live.onData) window.Live.onData(function () { render(); });
    return ctl;
  }

  /* ---------- first run (8A) ---------- */
  var PICKS = [['1.1.1.1', 'one.one.one.one'], ['8.8.8.8', 'dns.google']];
  // Seconds between rounds for the text: o.interval (a number, or a function such as WTH.interval), otherwise the
  // header's choice when the header is on the page, otherwise the default.
  function ivOf(o) {
    var v = o && o.interval, H = window.WTH;
    if (typeof v === 'function') { try { v = v(); } catch (e) { v = null; } }
    if (!(v > 0) && H && typeof H.interval === 'function') { try { v = H.interval(); } catch (e) { v = null; } }
    return v > 0 ? +v : WT.intervals[2];
  }
  function emptyText(iv) {
    var e = every(iv);
    return ['Введите имя или IP-адрес в поле сверху. Whilo Trace будет опрашивать путь до цели ' + e[0] + ' и покажет потери и задержку на каждом хопе.',
      'Type a name or IP address in the field above. Whilo Trace will probe the path to it ' + e[1] + ' and show loss and latency at every hop.'];
  }
  function emptyHtml(o) {
    var tx = emptyText(ivOf(o));
    return '<div class="es">' + I.route + '<h2 data-en="Add your first target">' + esc(pick('Добавьте первую цель', 'Add your first target')) + '</h2>' +
      L(tx[0], tx[1], 'es-text', 'p') +
      '<div class="es-picks">' + L('Для начала:', 'To start:') + PICKS.map(function (p) {
        return '<button type="button" class="btn es-pick" data-host="' + esc(p[0]) + '">' + I.plus + mono(p[0]) + '<span class="es-aside" data-en="' + esc(p[1]) + '">' + esc(p[1]) + '</span></button>';
      }).join('') + '</div>' +
      L('Свой роутер и хоп провайдера потом можно вынести из трассы цели отдельными таймлайнами.', 'Later you can pin your router and the provider\'s hop from a target\'s trace as separate timelines.', 'es-note', 'p') + '</div>';
  }
  // A quick-start button adds through o.onPick(host), else through the header (WTH.add): a promise, and that path
  // shows its own errors under the add field. Without either, add_target here, errors under the buttons.
  function bindEmpty(c) {
    var es = c.es;
    es.addEventListener('click', function (e) {
      var b = e.target.closest('.es-pick');
      if (!b || b.getAttribute('aria-busy') === 'true') return;
      var host = b.dataset.host, note = es.querySelector('.es-err'), H = window.WTH, add = c.o.onPick || (H && typeof H.add === 'function' ? H.add : null), p;
      if (note) note.remove();
      try { p = add ? add(host) : window.API.call('add_target', { req: { host: host, intervalS: ivOf(c.o) } }); } catch (err) { p = Promise.reject(err); }
      if (!p || typeof p.then !== 'function') return;
      b.setAttribute('aria-busy', 'true');
      p.then(function () {
        b.removeAttribute('aria-busy');
        // The message goes away with the first target: the caret goes to the add field, unless the screen moved it.
        if (document.activeElement === b && H && typeof H.focusField === 'function') H.focusField();
      }, function (err) {
        b.removeAttribute('aria-busy');
        if (add) return;
        var x = errPairs(err, { host: host });
        es.insertAdjacentHTML('beforeend', '<p class="es-note es-err" role="alert">' + L(x.text[0], x.text[1]) + '</p>');
      });
    });
  }
  // The first-run message in el. o: { onPick(host), interval } (see ivOf and bindEmpty). A second call on the same el
  // takes new options and updates the text in place. When the interval comes from a function or from the header,
  // the text follows it: it is read again after any click or key on the page (the header changes it on one).
  function empty(el, o) {
    var c = el._he;
    if (c && c.kind === 'empty' && c.es.parentNode === el) { if (o) c.o = o; c.update(); return c.es; }
    c = { kind: 'empty', el: el, o: o || {}, es: null, iv: null };
    el._he = c;
    el.innerHTML = emptyHtml(c.o);
    c.es = el.querySelector('.es');
    c.iv = ivOf(c.o);
    c.update = function () {
      var iv = ivOf(c.o), p = c.es.querySelector('.es-text'), tx;
      if (iv === c.iv || !p) return;
      c.iv = iv; tx = emptyText(iv);
      setText(p, tx[0], tx[1]);
    };
    bindEmpty(c);
    var follow = function () {
      if (el._he !== c || !c.es.isConnected) { document.removeEventListener('click', follow); document.removeEventListener('keyup', follow); return; }
      c.update();
    };
    document.addEventListener('click', follow);
    document.addEventListener('keyup', follow);
    return c.es;
  }

  /* ---------- error codes (docs/ui-contract.md) ---------- */
  // A code from the core, alone or with a detail after a colon («probe_unavailable: <detail>», «io: <detail>»);
  // the engine's own messages are read too («cannot resolve ...», «cannot open ICMP: ...», «probe to ... failed: ...»).
  function probeKey(name) {
    var n = String(name || '').toLowerCase(), k;
    for (k in WT.probes) if (k === n || WT.probes[k].label.toLowerCase() === n) return k;
    return null;
  }
  function parseCode(code) {
    var raw = String(code == null ? '' : typeof code === 'object' ? (code.code || code.message || '') : code).trim();
    var out = { raw: raw, code: raw, detail: '', host: '', probe: null }, m = /^([a-z][a-z0-9_]*)\s*:\s*([\s\S]*)$/.exec(raw), d;
    if (m) { out.code = m[1]; out.detail = m[2].trim(); }
    d = m ? out.detail : raw;
    if ((m = /^cannot resolve\s+([^:\s]+)/i.exec(d))) { out.code = 'resolve_failed'; out.host = m[1]; out.detail = ''; }
    else if ((m = /^cannot open\s+([^:]+?)\s*(?::\s*([\s\S]*))?$/i.exec(d))) { out.code = 'probe_unavailable'; out.probe = probeKey(m[1]); out.detail = (m[2] || '').trim(); }
    else if ((m = /^probe to\s+\S+\s+failed\s*(?::\s*([\s\S]*))?$/i.exec(d))) { out.code = 'probe_failed'; out.detail = (m[1] || '').trim(); }
    else if (out.code === 'probe_unavailable' && probeKey(out.detail)) { out.probe = probeKey(out.detail); out.detail = ''; }
    return out;
  }
  function dot(s) { return /[.!?…]$/.test(s) ? s : s + '.'; }
  // [ru, en] texts of an error: status (short, as a row's status), title (what happened), hint (what to do),
  // text (title and hint). ctx: { host, probe, interval (a row's, s), retryS (how often the screen asks again) }.
  function errPairs(code, ctx) {
    ctx = ctx || {};
    var e = parseCode(code), c = e.code, d = e.detail, host = ctx.host || e.host || '', probe = ctx.probe || e.probe;
    var P = probe ? probeLabel({ probe: probe }) : '', iv = ctx.interval > 0 ? every(ctx.interval) : null;
    var out = { code: c, detail: d, status: ['ошибка', 'Error'], title: ['', ''], hint: ['', ''] };
    if (c === 'invalid_host') {
      out.status = ['неверный адрес', 'Invalid address'];
      out.title = ['Это не похоже на имя или IP-адрес' + (host ? ': ' + host : '') + '.', 'This does not look like a name or an IP address' + (host ? ': ' + host : '') + '.'];
      out.hint = ['Введите имя, например example.com, или адрес IPv4, например 1.1.1.1.', 'Type a name such as example.com or an IPv4 address such as 1.1.1.1.'];
    } else if (c === 'resolve_failed') {
      out.status = ['не удалось найти хост', 'Host not found'];
      out.title = [host ? 'Имя ' + host + ' не нашлось в DNS.' : 'Имя не нашлось в DNS.', host ? 'The name ' + host + ' was not found in DNS.' : 'The name was not found in DNS.'];
      out.hint = iv ? ['Проверьте написание. Whilo Trace пробует снова ' + iv[0] + '.', 'Check the spelling. Whilo Trace retries ' + iv[1] + '.'] : ['Проверьте написание.', 'Check the spelling.'];
    } else if (c === 'no_ipv4') {
      out.status = ['нет адреса IPv4', 'No IPv4 address'];
      out.title = host ? ['Имя ' + host + ' нашлось, но у него только адреса IPv6.', 'The name ' + host + ' resolves, but only to IPv6 addresses.']
        : ['У этого имени только адреса IPv6.', 'This name resolves only to IPv6 addresses.'];
      out.hint = ['Whilo Trace пока работает только с IPv4.', 'Whilo Trace works with IPv4 only for now.'];
    } else if (c === 'probe_unavailable' && /permi|denied|eperm|eacces/i.test(d)) {
      // The system does not let the app open the sockets of this probe type.
      out.status = P ? ['нет прав на ' + P + '-сокеты', 'No ' + P + ' socket permission'] : ['нет прав на сокеты проб', 'No probe socket permission'];
      out.title = P ? ['Не опрашивается, пока нет прав на ' + P + '-сокеты.', 'Not probed until ' + P + ' sockets are allowed.']
        : ['Не опрашивается, пока нет прав на сокеты проб.', 'Not probed until probe sockets are allowed.'];
    } else if (c === 'probe_unavailable') {
      var avail = WT.probesAvailable.filter(function (p) { return p !== probe; }).map(function (p) { return probeLabel({ probe: p }); });
      out.status = P ? ['пробы ' + P + ' недоступны', P + ' probes unavailable'] : ['пробы недоступны', 'Probes unavailable'];
      out.title = d ? [(P ? 'Пробы ' + P : 'Пробы') + ' не отправляются: ' + dot(d), (P ? P + ' probes' : 'Probes') + ' cannot be sent: ' + dot(d)]
        : P ? ['Пробы ' + P + ' пока недоступны.', P + ' probes are not available yet.'] : ['Этот тип пробы пока недоступен.', 'This probe type is not available yet.'];
      if (!d && avail.length) out.hint = ['Доступны: ' + andList(avail, false) + '.', 'Available: ' + andList(avail, true) + '.'];
    } else if (c === 'probe_failed') {
      out.status = ['пробы не отправляются', 'Probes fail'];
      out.title = d ? ['Пробы не отправляются: ' + dot(d), 'Probes cannot be sent: ' + dot(d)] : ['Пробы не отправляются.', 'Probes cannot be sent.'];
    } else if (c === 'invalid_interval') {
      var lo = WT.intervals[0], hi = WT.intervals[WT.intervals.length - 1];
      out.status = ['неверный интервал', 'Invalid interval'];
      out.title = ['Интервал выбирается из списка: от ' + num1(lo, false) + ' до ' + num1(hi, false) + NB + 'с.', 'The interval is picked from the list: ' + num1(lo, true) + ' to ' + num1(hi, true) + ' s.'];
    } else if (c === 'no_such_target') {
      out.status = ['цели уже нет', 'Target is gone'];
      out.title = ['Этой цели уже нет.', 'This target is gone.'];
    } else if (c === 'no_such_hop') {
      out.status = ['хопа нет в трассе', 'Hop not in the trace'];
      out.title = ['Этого хопа нет в трассе или у него нет адреса.', 'This hop is not in the trace or has no address.'];
    } else if (c === 'not_implemented') {
      out.status = ['не работает в этой сборке', 'Not in this build'];
      out.title = ['Это пока не работает в этой сборке.', 'This does not work in this build yet.'];
    } else if (c === 'core_unavailable') {
      out.status = ['нет данных', 'No data'];
      out.title = ['Данные пока не пришли.', 'The data has not come in yet.'];
      out.hint = ctx.retryS > 0 ? ['Whilo Trace пробует снова ' + every(ctx.retryS)[0] + '.', 'Whilo Trace tries again ' + every(ctx.retryS)[1] + '.']
        : ['Whilo Trace пробует снова.', 'Whilo Trace keeps trying.'];
    } else if (c === 'db') {
      out.status = ['ошибка истории', 'History error'];
      out.title = ['Не удалось прочитать или записать историю' + (d ? ': ' + dot(d) : '.'), 'Could not read or write the history' + (d ? ': ' + dot(d) : '.')];
    } else if (c === 'io' && /^no documents folder/i.test(d)) {
      out.status = ['нет папки «Документы»', 'No Documents folder'];
      out.title = ['Не нашлась папка «Документы».', 'The Documents folder was not found.'];
      out.hint = ['Выберите папку для отчета.', 'Pick a folder for the report.'];
    } else if (c === 'io') {
      out.status = ['системная ошибка', 'System error'];
      out.title = d ? ['Системная ошибка: ' + dot(d), 'System error: ' + dot(d)] : ['Системная ошибка.', 'System error.'];
    } else {
      out.title = e.raw ? ['Ошибка: ' + e.raw, 'Error: ' + e.raw] : ['Неизвестная ошибка.', 'Unknown error.'];
    }
    out.text = [0, 1].map(function (i) { return out.title[i] + (out.hint[i] ? ' ' + out.hint[i] : ''); });
    return out;
  }
  // Texts of an error code in the interface language, for every part: { code, detail, status, title, hint, text }.
  // status is short, as a row's status («не удалось найти хост»); text is the whole message (title and hint).
  function errorInfo(code, ctx) {
    var e = errPairs(code, ctx), i = isEN() ? 1 : 0;
    return { code: e.code, detail: e.detail, status: e.status[i], title: e.title[i], hint: e.hint[i], text: e.text[i] };
  }
  // A row with t.error: { info (as errorInfo), status: HTML of the row's short status, body: HTML for the place of
  // its chart (the error well, without a scale) }.
  function rowError(t) {
    var ctx = { host: t.host, probe: t.probe, interval: t.interval }, e = errPairs(t.error, ctx);
    var icon = e.code === 'resolve_failed' ? I.nohost : e.code === 'no_ipv4' ? I.globe : e.code === 'probe_unavailable' ? I.lock : I.noreply;
    return {
      info: errorInfo(t.error, ctx),
      status: '<span class="er-st">' + L(e.status[0], e.status[1]) + '</span>',
      body: '<div class="er-well"><div class="er-msg">' + icon + '<div>' + L(e.title[0], e.title[1], 'er-msg-t', 'div') +
        (e.hint[0] ? L(e.hint[0], e.hint[1], 'er-msg-s', 'div') : '') + '</div></div></div>',
    };
  }

  /* ---------- report dialog (7A) ---------- */
  var FMT = [
    ['html', 'HTML', 'отчет для письма', 'report to send', 'Страница для письма: выводы, таймлайны и таблица хопов', 'Page to send: findings, timelines and hop table'],
    ['png', 'PNG', 'таймлайны', 'timelines', 'Картинка таймлайнов, как на экране', 'Timeline picture, as on screen'],
    ['csv', 'CSV', 'хопы и пробы, UTC', 'hops, probes, UTC', 'Таблица хопов и каждая проба; время в UTC', 'Hop table and every probe; times in UTC'],
    ['json', 'JSON', 'все данные, UTC', 'all data, UTC', 'Все данные отчета для разбора; время в UTC', 'All report data for analysis; times in UTC'],
  ];
  var FILES = { html: 1, png: 1, csv: 2, json: 1 }; // report.html, timeline.png, hops.csv + samples.csv, report.json
  var PRESETS = [['screen', 'Как на экране', 'On screen'], ['sel', 'Выделенное', 'Selection'], ['1h', '1 ч', '1 h'], ['6h', '6 ч', '6 h'], ['24h', '24 ч', '24 h'], ['custom', 'Свой', 'Custom']];
  var PRESET_MS = { '1h': 3600000, '6h': 21600000, '24h': DAY };
  var WIN_SEP = /Windows/i.test(navigator.userAgent) ? '\\' : '/';

  // Russian names for the city part of IANA zones. In Russian, Intl names most zones by a country or a region
  // («Азербайджан», «Центральная Европа») and some by another city (Asia/Novosibirsk: «Красноярск»), so a short list;
  // a zone outside it shows the offset only.
  var CITY_RU = {
    Baku: 'Баку', Tbilisi: 'Тбилиси', Yerevan: 'Ереван', Moscow: 'Москва', Kaliningrad: 'Калининград', Samara: 'Самара',
    Volgograd: 'Волгоград', Saratov: 'Саратов', Astrakhan: 'Астрахань', Ulyanovsk: 'Ульяновск', Kirov: 'Киров',
    Yekaterinburg: 'Екатеринбург', Omsk: 'Омск', Novosibirsk: 'Новосибирск', Barnaul: 'Барнаул', Tomsk: 'Томск',
    Novokuznetsk: 'Новокузнецк', Krasnoyarsk: 'Красноярск', Irkutsk: 'Иркутск', Chita: 'Чита', Yakutsk: 'Якутск',
    Vladivostok: 'Владивосток', Magadan: 'Магадан', Kamchatka: 'Камчатка', Minsk: 'Минск', Kyiv: 'Киев', Kiev: 'Киев',
    Chisinau: 'Кишинев', Riga: 'Рига', Vilnius: 'Вильнюс', Tallinn: 'Таллин', Almaty: 'Алматы', Tashkent: 'Ташкент',
    Bishkek: 'Бишкек', Dushanbe: 'Душанбе', Ashgabat: 'Ашхабад', Istanbul: 'Стамбул', London: 'Лондон', Berlin: 'Берлин',
    Paris: 'Париж', Amsterdam: 'Амстердам', Warsaw: 'Варшава', Prague: 'Прага', Vienna: 'Вена', Rome: 'Рим',
    Madrid: 'Мадрид', Helsinki: 'Хельсинки', Stockholm: 'Стокгольм', Dubai: 'Дубай', Tokyo: 'Токио',
    New_York: 'Нью-Йорк', Chicago: 'Чикаго', Los_Angeles: 'Лос-Анджелес', Toronto: 'Торонто',
  };
  // Place of the system time zone: the city of its IANA name, in English as it is (Asia/Baku: Baku).
  function zoneCity(en) {
    try {
      var z = Intl.DateTimeFormat().resolvedOptions().timeZone || '', city = z.split('/').pop();
      if (!z || /^(Etc\/)?(UTC|GMT)/.test(z)) return '';
      return en ? city.replace(/_/g, ' ') : CITY_RU[city] || '';
    } catch (e) { return ''; }
  }
  function tzLabel(o, en) {
    if (o.tz === 'utc') return 'UTC';
    var c = zoneCity(en);
    return offText(offMin(o.now)) + (c ? ', ' + c : '');
  }
  // Text of a time field: «27.10.2026 08:30» in Russian, «2026-10-27 08:30» in English; seconds when set.
  function inputText(ms, tz, en) {
    var f = fields(ms, tz), t = pad(f.h) + ':' + pad(f.mi) + (f.s ? ':' + pad(f.s) : '');
    return (en ? f.y + '-' + pad(f.mo) + '-' + pad(f.d) : pad(f.d) + '.' + pad(f.mo) + '.' + f.y) + ' ' + t;
  }
  function parseInput(s, tz) {
    var m = /^\s*(\d{1,2})\.(\d{1,2})\.(\d{4})[\s,T]+(\d{1,2}):(\d{2})(?::(\d{2}))?\s*$/.exec(s), y, mo, d;
    if (m) { d = +m[1]; mo = +m[2]; y = +m[3]; } else {
      m = /^\s*(\d{4})[-‑./](\d{1,2})[-‑./](\d{1,2})[\s,T]+(\d{1,2}):(\d{2})(?::(\d{2}))?\s*$/.exec(s);
      if (!m) return NaN;
      y = +m[1]; mo = +m[2]; d = +m[3];
    }
    var h = +m[4], mi = +m[5], se = +(m[6] || 0);
    if (mo < 1 || mo > 12 || d < 1 || h > 23 || mi > 59 || se > 59) return NaN;
    var utc = Date.UTC(y, mo - 1, d, h, mi, se);
    if (new Date(utc).getUTCDate() !== d) return NaN;
    return tz === 'utc' ? utc : new Date(y, mo - 1, d, h, mi, se).getTime();
  }
  function checkCustom(o) {
    var a = parseInput(o.text[0], o.tz), b = parseInput(o.text[1], o.tz);
    if (isNaN(a) || isNaN(b)) return ['Проверьте даты периода: ДД.ММ.ГГГГ ЧЧ:ММ', 'Check the period dates: YYYY-MM-DD HH:MM'];
    if (b <= a) return ['Начало периода должно быть раньше конца', 'The period must start before it ends'];
    if (b - a > DAY) return ['Период не длиннее 24 ч', 'The period can be 24 h at most'];
    if (b > nowMs() + MIN) return ['Конец периода позже текущего момента', 'The period ends after now'];
    o.custom = [a, b];
    return null;
  }
  // A period picked on a timeline: [fromMs, toMs), the end not included.
  function normRange(r) { return r && r.length === 2 && r[1] > r[0] ? [r[0], r[1]] : null; }
  function sameRange(a, b) { return !!a && !!b && Math.abs(a[0] - b[0]) < 1 && Math.abs(a[1] - b[1]) < 1; }
  // The period the data on screen is for (the asked one may still be on its way).
  function liveRange() { var v = window.Live, s = v && (v.shown || v.state); return s ? normRange(s.range) : null; }

  function periodRange(o) {
    if (o.period === 'sel') return o.sel;
    if (PRESET_MS[o.period]) return [o.now - PRESET_MS[o.period], o.now];
    if (o.period === 'custom') return o.cerr ? null : o.custom;
    return o.screen;
  }
  // Rows of the trace screen: the target's probe types and the hops pinned from its trace.
  function traceRows(o) {
    var t = tg(o.target);
    if (!t) return { t: null, probes: [], pinned: [] };
    var probes = WT.targets.filter(function (x) { return !x.hopOf && x.host === t.host; }), ids = {};
    probes.forEach(function (p) { ids[p.id] = 1; });
    return { t: t, probes: probes, pinned: WT.targets.filter(function (x) { return x.hopOf && ids[x.hopOf]; }) };
  }
  // Rows that go into the report: picked targets and hops (main screen), picked probe types and pinned hops (trace).
  function lines(o) {
    if (o.from === 'trace') { var r = traceRows(o); return r.probes.filter(function (p) { return !o.off[p.id]; }).concat(o.off.pinned ? [] : r.pinned); }
    return WT.targets.filter(function (t) { return !o.off[t.id]; });
  }
  // The data of the period: the screen's own snapshot for «On screen», otherwise a snapshot for the period
  // (the window that holds it, ending at its end, with range for exact numbers). It does not touch the screen.
  function dataReq(o) {
    var r = periodRange(o);
    if (!r) return null;
    if (o.period === 'screen' || o.period === 'sel') return { window: o.win, endMs: Math.round(o.screen[1]), range: o.period === 'sel' ? [Math.round(r[0]), Math.round(r[1])] : null, trace: null, buckets: o.N };
    if (PRESET_MS[o.period]) return { window: o.period, endMs: Math.round(o.now), range: null, trace: null, buckets: o.N };
    return { window: winFor(r[1] - r[0]), endMs: Math.round(r[1]), range: [Math.round(r[0]), Math.round(r[1])], trace: null, buckets: o.N };
  }
  function seed(o) {
    var scr = o.period === 'screen' && Math.abs(WT.toMs - o.screen[1]) < 1;
    var sel = o.period === 'sel' && sameRange(o.sel, liveRange()) && WT.targets.some(function (t) { return t.rangeStats; });
    return scr || sel ? { N: WT.N, fromMs: WT.fromMs, toMs: WT.toMs, bucketMs: WT.bucketMs, targets: WT.targets, history: WT.history } : null;
  }
  function rowData(o, id) {
    var list = (o.data && o.data.targets) || [];
    for (var i = 0; i < list.length; i++) if (list[i].id === id) return list[i];
    return null;
  }
  function statsOf(o, t) { return (o.useRange && t.rangeStats) || t.stats || WT.stats(t); }
  function roundsOf(t, s, r) { return s && isFinite(s.sent) && s.sent != null ? s.sent : Math.round((r[1] - r[0]) / 1000 / (t.interval || 2.5)); }
  function atAddr(h) { return h.addr ? ' (' + h.addr + ')' : ''; }

  // Findings in plain words from the numbers (stats) and the path summary (t.path), [ru, en].
  // Main screen: one line per target and probe type; trace screen: a paragraph per probe type.
  function finding(t, s, n, full) {
    var head = full ? '' : t.host + ', ' + probeLabel(t) + ': ', p = t.path || {}, lossy = s && s.lossPct > 0;
    if (!n) return full ? ['За этот период данных нет.', 'No data for this period.'] : [head + 'за этот период данных нет.', head + 'no data for this period.'];
    var r = cnt(n, ['раунд', 'раунда', 'раундов'], ['round', 'rounds']), o = lossy && p.origin ? p.origin : null;
    if (!full) {
      if (!lossy) return [head + 'потерь до цели нет за ' + r[0] + '.', head + 'no loss to the destination over ' + r[1] + '.'];
      return [head + 'потери до цели ' + pc(s.lossPct, false) + ' за ' + r[0] + (o ? ', начинаются на хопе ' + o.ttl + atAddr(o) : '') + '.',
        head + pc(s.lossPct, true) + ' loss to the destination over ' + r[1] + (o ? ', starting at hop ' + o.ttl + atAddr(o) : '') + '.'];
    }
    var ru = [lossy ? 'Потери до цели ' + pc(s.lossPct, false) + ' за ' + r[0] + '.' : 'Потерь до цели нет за ' + r[0] + '.'];
    var en = [lossy ? pc(s.lossPct, true) + ' loss to the destination over ' + r[1] + '.' : 'No loss to the destination over ' + r[1] + '.'];
    if (o) { ru.push('Потери начинаются на хопе ' + o.ttl + atAddr(o) + ' и продолжаются до цели.'); en.push('Loss starts at hop ' + o.ttl + atAddr(o) + ' and carries on to the destination.'); }
    var sil = p.silent || [], label = probeLabel(t);
    if (sil.length === 1) { ru.push('Хоп ' + sil[0] + ' не отвечает на ' + label + ', хопы дальше отвечают.'); en.push('Hop ' + sil[0] + ' does not answer ' + label + '; later hops do.'); }
    if (sil.length > 1) { ru.push('Хопы ' + andList(sil, false) + ' не отвечают на ' + label + ', хопы дальше отвечают.'); en.push('Hops ' + andList(sil, true) + ' do not answer ' + label + '; later hops do.'); }
    (p.partial || []).forEach(function (h) { ru.push('Хоп ' + h.ttl + atAddr(h) + ' отвечает не на все пробы.'); en.push('Hop ' + h.ttl + atAddr(h) + ' answers only some probes.'); });
    return [ru.join(' '), en.join(' ')];
  }
  // [{ t, f: [ru, en] }] for the target rows of the report.
  function findingsOf(o) {
    var r = periodRange(o), full = o.from === 'trace';
    if (!o.data || !r) return [];
    return lines(o).filter(function (t) { return !t.hopOf; }).map(function (t0) {
      var t = rowData(o, t0.id) || t0, s = statsOf(o, t);
      return { t: t, f: finding(t, s, roundsOf(t, s, r), full) };
    });
  }

  function fsBlock(ru, en, body, extra, cls) {
    return '<section class="rp-fs' + (cls ? ' ' + cls : '') + '"><div class="rp-fs-head">' + L(ru, en, 'rp-lbl') + (extra || '') + '</div>' + body + '</section>';
  }
  function closeBtn() {
    var c = pick('Закрыть', 'Close');
    return '<button type="button" class="btn btn-ghost btn-icon btn-sm" data-act="close" aria-label="' + esc(c) + '" title="' + esc(c) + '">' + I.close + '</button>';
  }
  function dlgHead(ru, en, subHtml) {
    return '<div class="rp-head"><div><h2 data-en="' + esc(en) + '">' + esc(isEN() ? en : ru) + '</h2>' + (subHtml || '') + '</div>' + closeBtn() + '</div>';
  }
  function toggle(act, key, on, inner) {
    return '<button type="button" class="rp-tog" data-act="' + act + '" data-key="' + esc(key) + '" aria-pressed="' + on + '"><span class="ck">' + I.check + '</span>' + inner + '</button>';
  }
  function checkRow(act, on, ru, en) {
    return '<div class="rp-row" role="checkbox" tabindex="0" data-act="' + act + '" aria-checked="' + on + '"><span class="ck">' + I.check + '</span>' + L(ru, en) + '</div>';
  }
  function mainTargets(o) {
    var rows = WT.targets, all = rows.every(function (t) { return !o.off[t.id]; });
    var items = rows.map(function (t) {
      var label = t.hopOf ? L(nm(t, false), nm(t, true)) : mono(t.host);
      var aside = t.hopOf ? L('хоп ' + t.hop, 'hop ' + t.hop, 'rp-aside') : '<span class="rp-aside" data-en="' + esc(probeLabel(t)) + '">' + esc(probeLabel(t)) + '</span>';
      return toggle('tg', t.id, !o.off[t.id], label + aside);
    }).join('');
    var btn = rows.length ? '<button type="button" class="btn btn-ghost btn-sm" data-act="tg-all">' + (all ? L('Снять все', 'Clear all') : L('Выбрать все', 'Select all')) + '</button>' : '';
    return fsBlock('Цели', 'Targets', '<div class="rp-tg">' + (items || L('Целей пока нет', 'No targets yet', 'rp-aside')) + '</div>', btn);
  }
  // The target under the dialog title (trace screen).
  function traceSub(o) {
    var t = traceRows(o).t;
    if (!t) return '';
    var n = (t.path && t.path.hops) || (WT.trace && WT.trace.hops ? WT.trace.hops.length : 0);
    var hops = n ? cnt(n, ['хоп', 'хопа', 'хопов'], ['hop', 'hops']) : null, name = t.name && t.name !== t.host ? t.name + ' · ' : '';
    return '<p class="rp-sub rp-target">' + mono(t.host) + L(name + 'трасса' + (hops ? ', ' + hops[0] : ''), name + 'trace' + (hops ? ', ' + hops[1] : ''), 'rp-aside') + '</p>';
  }
  function traceTarget(o) {
    var r = traceRows(o), pins = r.pinned;
    var body = '<div class="rp-line rp-first">' + L('Типы проб', 'Probe types', 'rp-k') + '<div class="rp-tg">' +
      r.probes.map(function (p) { return toggle('pr', p.id, !o.off[p.id], '<span data-en="' + esc(probeLabel(p)) + '">' + esc(probeLabel(p)) + '</span>'); }).join('') + '</div></div>';
    if (pins.length) {
      var ru = pins.map(function (h) { return nm(h, false) + ' (хоп ' + h.hop + ')'; }).join(', '), en = pins.map(function (h) { return nm(h, true) + ' (hop ' + h.hop + ')'; }).join(', ');
      body += '<div class="rp-pins">' + checkRow('pin', !o.off.pinned, 'Таймлайны вынесенных хопов: ' + ru, 'Timelines of pinned hops: ' + en) + '</div>';
    }
    return fsBlock('Цель', 'Target', body);
  }
  function selHint(o) {
    return o.from === 'trace' ? pick('Сначала выделите участок на таймлайне хопа', 'Select a range on the hop timeline first')
      : pick('Сначала выделите участок на таймлайне', 'Select a range on a timeline first');
  }
  // The day around the period: the day the strip shows, when the period is in it.
  function lanePage(a, b) {
    var s = stripOn(), p = s && s.st.pg, now = nowMs();
    if (p && a >= p.a - 1000 && b <= p.b + 1000) return p;
    var end = Math.max(b, Math.min(now, a + DAY)), live = end >= now - 1000;
    return { a: (live ? now : end) - DAY, b: live ? now : end, live: live };
  }
  function miniLane(o, a, b) {
    var pg = lanePage(a, b), mk = marksFor(pg.a, pg.b, windowMarks(o.data));
    if (!mk.covered) fetchDay(pg.b, pg.live, o.soft);
    var l = clamp((a - pg.a) / DAY * 100, 0, 100), w = clamp((b - pg.a) / DAY * 100, 0, 100) - l;
    var pos = 'left:' + l + '%;width:' + w + '%', tk = tickLabels([pg.a, pg.a + DAY / 2, pg.b], o.tz);
    return '<div class="hx-lane rp-lane" aria-hidden="true"><div class="hx-clip">' + GRID + '<i class="hx-sel" style="' + pos + '"></i>' + markHtml(mk.marks, pg.a) + '</div>' +
      '<i class="hx-frame" data-w="' + (w / 100 * DAY) + '" style="' + pos + '"></i></div>' +
      '<div class="rp-lane-ticks">' + tk.map(function (p, k) {
        return '<span class="num" style="left:' + (k * 50) + '%" data-en="' + esc(p[1]) + '">' + esc(isEN() ? p[1] : p[0]) + '</span>';
      }).join('') + '</div>';
  }
  function rangeLine(o, r) {
    var len = r[1] - r[0], x = [' · ' + dur(len, false), ' · ' + dur(len, true)];
    if (o.from === 'trace' && o.data) {
      var p = traceRows(o).probes.filter(function (q) { return !o.off[q.id]; })[0], t = p && (rowData(o, p.id) || p);
      if (t) { var c = cnt(roundsOf(t, statsOf(o, t), r), ['раунд', 'раунда', 'раундов'], ['round', 'rounds']); x[0] += ' · ' + c[0]; x[1] += ' · ' + c[1]; }
    }
    return '<span class="rp-sum-t">' + L(rangeText(r[0], r[1], o.tz, false), rangeText(r[0], r[1], o.tz, true), 'num') + L(x[0], x[1], 'rp-aside') + '</span>';
  }
  function customLine(o) {
    var ph = pick('ДД.ММ.ГГГГ ЧЧ:ММ', 'YYYY-MM-DD HH:MM');
    var input = function (k, i, label) {
      return '<input type="text" class="rp-in num" data-act="' + k + '" value="' + esc(o.text[i]) + '" placeholder="' + esc(ph) + '" aria-label="' + esc(label) + '" spellcheck="false" autocomplete="off">';
    };
    var d = o.cerr ? '' : '· ' + dur(o.custom[1] - o.custom[0], isEN());
    return '<span class="rp-sum-t is-custom">' + L('с', 'from', 'rp-aside') + input('cfrom', 0, pick('Начало периода', 'Period start')) +
      L('по', 'to', 'rp-aside') + input('cto', 1, pick('Конец периода', 'Period end')) + '<span class="rp-aside rp-cdur">' + esc(d) + '</span></span>';
  }
  function tzButton(o) {
    var title = pick('Часовой пояс для HTML, PNG и имени папки; CSV и JSON пишут время в UTC', 'Time zone for HTML, PNG and the folder name; CSV and JSON use UTC');
    var menu = !o.tzMenu ? '' : '<div class="rp-pmenu to-right" role="listbox" aria-label="' + esc(pick('Часовой пояс', 'Time zone')) + '">' +
      [['sys', tzLabel({ tz: 'sys', now: o.now }, false), tzLabel({ tz: 'sys', now: o.now }, true)], ['utc', 'UTC', 'UTC']].map(function (x) {
        var on = o.tz === x[0];
        return '<button type="button" role="option" data-act="tz-pick" data-key="' + x[0] + '" aria-selected="' + on + '"><span class="rp-pm-ck">' + (on ? I.check : '') + '</span>' + L(x[1], x[2]) + '</button>';
      }).join('') + '</div>';
    return '<span class="rp-pwrap"><button type="button" class="sel-btn" data-act="tz" aria-haspopup="listbox" aria-expanded="' + !!o.tzMenu + '" title="' + esc(title) + '">' +
      L(tzLabel(o, false), tzLabel(o, true)) + I.chev + '</button>' + menu + '</span>';
  }
  function sumHtml(o) {
    var r = periodRange(o);
    return o.period === 'custom' ? customLine(o) : r ? rangeLine(o, r) : '';
  }
  function periodBlock(o) {
    var r = periodRange(o) || o.custom;
    var seg = '<div class="seg" role="group" aria-label="' + esc(pick('Период', 'Period')) + '">' + PRESETS.map(function (x) {
      var dis = x[0] === 'sel' && !o.sel;
      return '<button type="button" data-act="period" data-key="' + x[0] + '" aria-pressed="' + (x[0] === o.period) + '"' + (dis ? ' aria-disabled="true" title="' + esc(selHint(o)) + '"' : '') + '>' + L(x[1], x[2]) + '</button>';
    }).join('') + '</div>';
    return fsBlock('Период', 'Period', seg + '<div class="rp-lanebox">' + miniLane(o, r[0], r[1]) + '</div>' +
      '<div class="rp-sum"><span class="rp-sumbox">' + sumHtml(o) + '</span>' + tzButton(o) + '</div>');
  }
  function tiles(o) {
    return '<div class="rp-tiles">' + FMT.map(function (f) {
      return '<button type="button" class="rp-tile" data-act="fmt" data-key="' + f[0] + '" aria-pressed="' + !!o.fmt[f[0]] + '" title="' + esc(pick(f[4], f[5])) + '">' +
        '<span class="rp-tile-top"><span class="ck">' + I.check + '</span><span data-en="' + f[1] + '">' + f[1] + '</span></span>' + L(f[2], f[3], 'rp-tile-d') + '</button>';
    }).join('') + '</div>';
  }
  function findingsBox(o) {
    if (!o.findings) return '';
    if (o.loadErr) { var e = errPairs(o.loadErr); return '<div class="rp-fnd"><p>' + L(e.title[0], e.title[1]) + '</p></div>'; }
    if (!periodRange(o)) return '';
    if (!o.data) return '<div class="rp-fnd"><p class="rp-wait">' + L('Считаю выводы…', 'Working out the findings…') + '</p></div>';
    var list = findingsOf(o);
    if (!list.length) return '';
    if (o.from === 'trace') return '<div class="rp-fnd">' + list.map(function (x) { return '<p>' + chip(x.t) + L(x.f[0], x.f[1]) + '</p>'; }).join('') + '</div>';
    var html = list.slice(0, 2).map(function (x) { return '<p>' + L(x.f[0], x.f[1]) + '</p>'; }).join(''), rest = list.length - 2;
    if (rest > 0) {
      var c = cnt(rest, ['цель', 'цели', 'целей'], ['target', 'targets']);
      html += '<p>' + L('и еще ' + c[0] + ' в отчете', 'and ' + c[1].replace(/ targets?$/, '') + ' more ' + (rest === 1 ? 'target' : 'targets') + ' in the report', 'rp-more') + '</p>';
    }
    return '<div class="rp-fnd">' + html + '</div>';
  }
  function whatBlock(o) {
    return fsBlock('Что войдет', 'What goes in', tiles(o) + '<div class="rp-fnd-row">' + checkRow('fnd', !!o.findings, 'Выводы простым текстом', 'Plain-text findings') + '</div>' +
      '<div class="rp-fndbox">' + findingsBox(o) + '</div>');
  }
  // Folder name as the core makes it: the target's address when the rows are of one target (its probe types and the
  // hops pinned from it), whilo-trace for more, and the period end in the report's time zone.
  function folderName(o) {
    var r = periodRange(o) || o.screen, seen = {}, hosts = [];
    lines(o).forEach(function (t) {
      var own = (t.hopOf && tg(t.hopOf)) || t, key = t.targetId != null ? t.targetId : own.host;
      if (!seen[key]) { seen[key] = 1; hosts.push(own.host); }
    });
    var base = hosts.length === 1 ? hosts[0].replace(/[^A-Za-z0-9.-]/g, '_') : 'whilo-trace';
    return base + '-' + stamp(r[1], o.tz);
  }
  function pathOf(o) {
    var name = folderName(o);
    if (o.dir) return [o.dir + WIN_SEP + name, o.dir + WIN_SEP + name];
    return ['Документы' + WIN_SEP + 'Whilo Trace' + WIN_SEP + name, 'Documents' + WIN_SEP + 'Whilo Trace' + WIN_SEP + name];
  }
  // The path: its beginning shrinks with an ellipsis, the folder name stays visible.
  function pathField(p) {
    var cut = function (x) { var i = Math.max(x.lastIndexOf('\\'), x.lastIndexOf('/')); return [x.slice(0, i + 1), x.slice(i + 1)]; };
    var r = cut(p[0]), e = cut(p[1]);
    return '<span class="rp-path-f" title="' + esc(isEN() ? p[1] : p[0]) + '">' + I.folder + L(r[0], e[0], 'mono rp-path-pre') + L(r[1], e[1], 'mono rp-path-name') + '</span>';
  }
  function folderBlock(o) {
    return fsBlock('Папка', 'Folder', '<div class="rp-path">' + pathField(pathOf(o)) +
      '<button type="button" class="btn" data-act="dir">' + L('Изменить…', 'Change…') + '</button></div>');
  }
  // The trace screen needs a probe type: the pinned hops alone are not a report.
  function fileCount(o) {
    var rows = lines(o);
    if (o.from === 'trace' && !rows.some(function (t) { return !t.hopOf; })) return { n: 0, why: ['Выберите хотя бы один тип пробы', 'Pick at least one probe type'] };
    if (!rows.length) return { n: 0, why: ['Выберите хотя бы одну цель', 'Pick at least one target'] };
    var n = 0;
    FMT.forEach(function (f) { if (o.fmt[f[0]]) n += FILES[f[0]]; });
    if (!n) return { n: 0, why: ['Выберите хотя бы один формат', 'Pick at least one format'] };
    if (o.period === 'custom' && o.cerr) return { n: 0, why: o.cerr };
    return { n: n };
  }
  // What the footer says and whether «Save report» works. While the period's data is on its way the button stays on:
  // a click then saves as soon as the data is in.
  function footState(o) {
    var f = fileCount(o);
    if (o.saving) return { msg: ['Сохраняю отчет…', 'Saving the report…'], warn: false, on: false };
    if (o.err) return { msg: o.err, warn: true, on: !f.why && !o.loadErr };
    if (f.why) return { msg: f.why, warn: true, on: false };
    if (o.loadErr) return { msg: ['Нет данных за период', 'No data for the period'], warn: true, on: false };
    if (!o.data) return { msg: ['Готовлю данные…', 'Getting the data…'], warn: false, on: true };
    return { msg: cnt(f.n, ['файл', 'файла', 'файлов'], ['file', 'files']), warn: false, on: true };
  }
  function footer(o) {
    var s = footState(o);
    return '<div class="rp-foot">' + L(s.msg[0], s.msg[1], 'rp-count' + (s.warn ? ' warn' : '')) +
      '<button type="button" class="btn btn-primary" data-act="save"' + (s.on ? '' : ' disabled') + '>' + L('Сохранить отчет', 'Save report') + '</button>' +
      '<button type="button" class="btn" data-act="close">' + L('Отмена', 'Cancel') + '</button></div>';
  }
  // The same footer in place: its buttons stay the same elements, so a click on its way is not lost.
  function patchFoot(foot, o) {
    var s = footState(o), m = foot.querySelector('.rp-count'), b = foot.querySelector('[data-act="save"]');
    if (!m || !b) { foot.outerHTML = footer(o); return; }
    setText(m, s.msg[0], s.msg[1]);
    m.classList.toggle('warn', s.warn);
    if (b.disabled !== !s.on) b.disabled = !s.on;
  }
  // One name for the feature in the whole interface: «Отчет для провайдера», «Report for ISP».
  function formHtml(o) {
    var trace = o.from === 'trace';
    return '<div class="rp" role="dialog" aria-modal="true" aria-label="' + esc(pick('Отчет для провайдера', 'Report for ISP')) + '">' +
      dlgHead('Отчет для провайдера', 'Report for ISP', trace ? traceSub(o) : '') +
      '<div class="rp-body">' + (trace ? traceTarget(o) : mainTargets(o)) + periodBlock(o) + whatBlock(o) + folderBlock(o) + '</div>' + footer(o) + '</div>';
  }
  // What was saved: the target and probe types (trace screen) or the number of targets and hops (main screen), and the period.
  function savedWhat(o) {
    var r = periodRange(o), tz = ' (' + tzLabel(o, false).split(',')[0] + ')';
    var when = [rangeText(r[0], r[1], o.tz, false) + tz, rangeText(r[0], r[1], o.tz, true) + tz];
    if (o.from === 'trace') {
      var t = traceRows(o).t, labels = lines(o).filter(function (x) { return !x.hopOf; }).map(probeLabel);
      var head = t ? t.host + (labels.length ? ', ' : '') : '';
      return [head + andList(labels, false) + ' · ' + when[0], head + andList(labels, true) + ' · ' + when[1]];
    }
    // Targets are counted by address, as in the summary: ICMP and TCP 443 to 1.1.1.1 are one target.
    var hosts = {}, nh = 0;
    lines(o).forEach(function (t) { if (t.hopOf) nh++; else hosts[t.host] = 1; });
    var a = cnt(Object.keys(hosts).length, ['цель', 'цели', 'целей'], ['target', 'targets']), b = cnt(nh, ['хоп', 'хопа', 'хопов'], ['hop', 'hops']);
    return [a[0] + (nh ? ', ' + b[0] : '') + ' · ' + when[0], a[1] + (nh ? ', ' + b[1] : '') + ' · ' + when[1]];
  }
  function savedHtml(o) {
    var res = o.res, files = res.files || [], dir = String(res.dir || ''), c = cnt(files.length, ['файл', 'файла', 'файлов'], ['file', 'files']);
    return '<div class="rp" role="dialog" aria-modal="true" aria-label="' + esc(pick('Отчет сохранен', 'Report saved')) + '">' +
      '<div class="rp-head"><div><div class="rp-ok">' + I.check + '<h2 data-en="Report saved">' + esc(pick('Отчет сохранен', 'Report saved')) + '</h2></div>' +
      L(o.what[0], o.what[1], 'rp-sub', 'p') + '</div>' + closeBtn() + '</div>' +
      '<div class="rp-body">' +
      fsBlock('Папка', 'Folder', '<div class="rp-path">' + pathField([dir, dir]) + '<button type="button" class="btn" data-act="copy">' + I.copy +
        (o.copied ? L('Скопировано', 'Copied') : L('Копировать путь', 'Copy path')) + '</button></div>') +
      fsBlock('Файлы', 'Files', '<div class="rp-files">' + files.map(function (f) { return mono(f); }).join('') + '</div>') +
      '</div><div class="rp-foot">' + (o.err ? L(o.err[0], o.err[1], 'rp-count warn') : L(c[0], c[1], 'rp-count')) +
      '<button type="button" class="btn btn-primary" data-act="open-dir">' + I.folder + L('Открыть папку', 'Open folder') + '</button>' +
      (files.indexOf('report.html') >= 0 ? '<button type="button" class="btn" data-act="open-file">' + L('Открыть отчет', 'Open report') + '</button>' : '') +
      '<button type="button" class="btn" data-act="close">' + L('Готово', 'Done') + '</button></div></div>';
  }

  /* ---------- timeline picture for the report (PNG) ---------- */
  // Light colours from tokens.css: the report goes to the provider and is printed, so it is always light.
  function lightColors() {
    var el = document.createElement('div'), out = {};
    el.setAttribute('data-theme', 'light');
    el.style.cssText = 'position:absolute;left:-9999px;top:0;visibility:hidden';
    document.body.appendChild(el);
    var cs = getComputedStyle(el);
    ['layer', 'well', 'control', 'title', 'text', 'muted', 'line', 'loss', 'loss-text', 'partial', 'zone-warn', 'zone-bad'].forEach(function (k) { out[k] = cs.getPropertyValue('--' + k).trim(); });
    el.remove();
    return out;
  }
  function rrect(g, x, y, w, h, r) {
    g.beginPath(); g.moveTo(x + r, y);
    g.arcTo(x + w, y, x + w, y + h, r); g.arcTo(x + w, y + h, x, y + h, r);
    g.arcTo(x, y + h, x, y, r); g.arcTo(x, y, x + w, y, r); g.closePath();
  }
  function drawPng(o) {
    var rows = lines(o), r = periodRange(o), d = o.data;
    if (!rows.length || !r || !d) return null;
    var en = isEN(), C = lightColors(), font = getComputedStyle(document.body).fontFamily || 'sans-serif', monoF = '"Cascadia Mono", ui-monospace, Consolas, monospace';
    var W = 1000, pad = 24, plotH = 72, block = 24 + plotH + 22, gap = 14, k = 2;
    var H = pad + 22 + 14 + rows.length * block + (rows.length - 1) * gap + pad;
    var cv = document.createElement('canvas');
    cv.width = W * k; cv.height = H * k;
    var g = cv.getContext('2d');
    g.scale(k, k);
    g.fillStyle = C.layer; g.fillRect(0, 0, W, H);
    var tz = tzLabel(o, en).split(',')[0];
    g.font = '600 16px ' + font; g.fillStyle = C.title; g.textAlign = 'left';
    g.fillText(pick('Задержка и потери', 'Latency and loss'), pad, pad + 16);
    var tw = g.measureText(pick('Задержка и потери', 'Latency and loss')).width;
    g.font = '13px ' + font; g.fillStyle = C.muted;
    g.fillText(rangeText(r[0], r[1], o.tz, en).replace(/‑/g, '-') + ' (' + tz + ') · Whilo Trace', pad + tw + 12, pad + 16);
    var sec = fields(r[0], o.tz).s || fields(r[1], o.tz).s, warn = (WT.settings && WT.settings.warnMs) || 100, bad = (WT.settings && WT.settings.badMs) || 200;
    var y = pad + 22 + 14, x0 = pad, pw = W - 2 * pad, len = r[1] - r[0];
    var X = function (ms) { return x0 + (clamp(ms, r[0], r[1]) - r[0]) / len * pw; };
    rows.forEach(function (t0, idx) {
      var t = rowData(o, t0.id) || t0, s = statsOf(o, t);
      // name, address and probe type, numbers on the right
      g.textAlign = 'left'; g.fillStyle = C.title;
      var name = t.hopOf ? nm(t, en) : t.host;
      g.font = (t.hopOf ? '600 14px ' + font : '600 14px ' + monoF);
      g.fillText(name, x0, y + 15);
      var x = x0 + g.measureText(name).width + 10;
      var sub = t.hopOf ? t.host + ' · ' + (en ? 'hop ' : 'хоп ') + t.hop : (t.name && t.name !== t.host ? t.name : '');
      if (sub) { g.font = '13px ' + (t.hopOf ? monoF : font); g.fillStyle = C.muted; g.fillText(sub, x, y + 15); x += g.measureText(sub).width + 10; }
      if (!t.hopOf) {
        var lb = probeLabel(t);
        g.font = '600 12px ' + font;
        var cw = g.measureText(lb).width + 14;
        rrect(g, x, y + 1, cw, 20, 6); g.fillStyle = C.control; g.fill();
        g.fillStyle = C.title; g.fillText(lb, x + 7, y + 15);
      }
      var parts = [];
      if (s) {
        parts.push([(en ? 'loss ' : 'потери ') + pc(s.lossPct || 0, en), s.lossPct > 0 ? C['loss-text'] : C.text]);
        if (s.avg != null) parts.push([(en ? 'avg ' : 'сред. ') + msTxt(s.avg, en), C.text]);
        if (s.max != null) parts.push([(en ? 'max ' : 'макс. ') + msTxt(s.max, en), C.text]);
      }
      g.font = '13px ' + font; g.textAlign = 'right';
      var rx = x0 + pw;
      for (var p = parts.length - 1; p >= 0; p--) {
        g.fillStyle = parts[p][1]; g.fillText(parts[p][0], rx, y + 15);
        rx -= g.measureText(parts[p][0]).width + (p ? 6 : 0);
        if (p) { g.fillStyle = C.muted; g.fillText('·', rx, y + 15); rx -= g.measureText('·').width + 6; }
      }
      // chart: threshold zones, loss columns, amber marks, latency line
      var py = y + 24, max = t.max || 10;
      var Y = function (v) { return py + plotH - 4 - Math.min(v, max) / max * (plotH - 8); };
      var Z = function (v) { return py + plotH - Math.min(v, max) / max * plotH; };
      g.save();
      rrect(g, x0, py, pw, plotH, 6); g.fillStyle = C.well; g.fill(); g.clip();
      if (max > warn) { g.fillStyle = C['zone-warn']; g.fillRect(x0, Z(Math.min(bad, max)), pw, Z(warn) - Z(Math.min(bad, max))); }
      if (max > bad) { g.fillStyle = C['zone-bad']; g.fillRect(x0, py, pw, Z(bad) - py); }
      var from = d.fromMs, bm = d.bucketMs, vals = WT.series(t.series || []), n = vals.length, lost = toSet(t.loss);
      var lr = runs((t.loss || []).slice().sort(byNum)), pr = runs((t.partial || []).slice().sort(byNum));
      g.fillStyle = C.loss;
      lr.forEach(function (q) { var a = X(from + q[0] * bm), b = X(from + (q[1] + 1) * bm); if (b > a) g.fillRect(a, py, Math.max(1, b - a), plotH); });
      g.fillStyle = C.partial;
      pr.forEach(function (q) { var a = X(from + q[0] * bm), b = X(from + (q[1] + 1) * bm); if (b > a) g.fillRect(a, py + plotH - 5, Math.max(1, b - a), 5); });
      g.beginPath();
      var pen = false;
      for (var i = 0; i < n; i++) {
        var ta = from + i * bm, tb = ta + bm;
        if (tb <= r[0] || ta >= r[1] || vals[i] == null || lost[i]) { pen = false; continue; }
        var px = X((ta + tb) / 2), yy = Y(vals[i]);
        if (pen) g.lineTo(px, yy); else g.moveTo(px, yy);
        pen = true;
      }
      g.strokeStyle = C.line; g.lineWidth = 2; g.lineJoin = 'round'; g.lineCap = 'round'; g.stroke();
      g.restore();
      // latency scale inside the field, as on screen: its top with the unit and zero, on a halo of the field's fill
      g.font = '12px ' + font; g.textAlign = 'left'; g.lineJoin = 'round'; g.lineWidth = 4; g.strokeStyle = C.well; g.fillStyle = C.muted;
      [[num1(max, en) + (en ? ' ms' : NB + 'мс'), py + 15], ['0', py + plotH - 6]].forEach(function (l) {
        g.strokeText(l[0], x0 + 8, l[1]); g.fillText(l[0], x0 + 8, l[1]);
      });
      // time axis: six labels from the start to the end of the period
      g.font = '12px ' + font; g.fillStyle = C.muted;
      for (var j = 0; j <= 5; j++) {
        var tt = r[0] + len * j / 5, lab = sec ? hms(tt, o.tz) : hm(tt, o.tz);
        g.textAlign = j === 0 ? 'left' : j === 5 ? 'right' : 'center';
        g.fillText(lab, X(tt), py + plotH + 16);
      }
      y += block + (idx < rows.length - 1 ? gap : 0);
    });
    return cv.toDataURL('image/png');
  }

  // The «Report for ISP» dialog in host (the screen's modal box, it is shown and hidden here).
  // opts: { from: 'main' | 'trace', target: row id (trace screen), range: [fromMs, toMs) picked on a timeline
  // (default: the period of the data on screen), onClose(result | null) }. The dialog asks the core for the period's
  // data, the folder (pick_folder), saves (export_report) and opens the folder (open_folder) itself.
  // While it is open it takes Esc and Tab for the whole page, so the screen under it needs no handler of its own.
  function report(host, opts) {
    opts = opts || {};
    if (host._rp && host._rp.close) host._rp.close(true);
    var from = opts.from === 'trace' ? 'trace' : 'main', target = null;
    if (from === 'trace') {
      var t0 = tg(opts.target || (WT.trace && WT.trace.target));
      if (t0 && t0.hopOf) t0 = tg(t0.hopOf);
      target = t0 ? t0.id : null;
    }
    var scr = [WT.fromMs, WT.toMs], sel = normRange(opts.range !== undefined ? opts.range : liveRange());
    var o = {
      from: from, target: target, now: nowMs(), screen: scr, win: winOf(scr[1] - scr[0]) || (window.Live && window.Live.state.window) || '10m',
      N: WT.N || 120, sel: sel, period: opts.period || (sel ? 'sel' : 'screen'), tz: 'sys', tzMenu: false,
      fmt: { html: true, png: true, csv: true, json: true }, findings: true, off: {}, dir: null,
      custom: [Math.round(scr[0] / MIN) * MIN, Math.round(scr[1] / MIN) * MIN], text: null, cerr: null,
      data: null, useRange: false, key: null, loadErr: null, saving: false, saveWanted: false, err: null, res: null, what: null, copied: false,
    };
    if (o.period === 'sel' && !o.sel) o.period = 'screen';
    o.text = [inputText(o.custom[0], o.tz, isEN()), inputText(o.custom[1], o.tz, isEN())];
    var closed = false, painting = false, typeT = 0, prev = document.activeElement;
    host.innerHTML = '<div class="ms"><div class="ms-scrim"></div><div class="rp-host"></div></div>';
    var root = host.firstChild, dlg = root.lastChild;
    var ctl = { kind: 'report', o: o, close: close };
    host._rp = ctl;
    o.soft = function () { if (!closed) soft(); };

    function focusAct(act, key) {
      var el = dlg.querySelector('[data-act="' + act + '"]' + (key ? '[data-key="' + key + '"]' : ''));
      if (el && !el.disabled) el.focus({ preventScroll: true });
      return el;
    }
    function remember() {
      var ae = document.activeElement;
      return ae && dlg.contains(ae) && ae.dataset && ae.dataset.act ? [ae.dataset.act, ae.dataset.key] : null;
    }
    function isTime(el) { var a = el && el.dataset && el.dataset.act; return a === 'cfrom' || a === 'cto'; }
    // The text of a time field into the state, nothing is drawn.
    function readField(input) {
      o.text[input.dataset.act === 'cfrom' ? 0 : 1] = input.value;
      o.cerr = checkCustom(o);
    }
    // The whole dialog again (a toggle, a new state). The typed text of a time field is already in the state (input
    // event); the change event that its removal fires is ignored, so a repaint never runs inside another one.
    function paint() {
      if (closed || painting) return;
      var f = remember();
      painting = true;
      try { dlg.innerHTML = o.res ? savedHtml(o) : formHtml(o); fixNarrow(dlg); } finally { painting = false; }
      if (f) focusAct(f[0], f[1]);
    }
    // The places that follow the data and the period (strip, findings, period line, folder, footer), in place:
    // typing, the focus and a click on its way stay.
    function soft() {
      if (closed || painting || o.res) return;
      var r = periodRange(o) || o.custom, lane = dlg.querySelector('.rp-lanebox'), fb = dlg.querySelector('.rp-fndbox');
      var sum = dlg.querySelector('.rp-sumbox'), path = dlg.querySelector('.rp-path-f'), foot = dlg.querySelector('.rp-foot');
      if (lane) { lane.innerHTML = miniLane(o, r[0], r[1]); fixNarrow(lane); }
      if (fb) fb.innerHTML = findingsBox(o);
      if (sum && o.period !== 'custom') sum.innerHTML = sumHtml(o);
      if (sum && o.period === 'custom') { var cd = sum.querySelector('.rp-cdur'); if (cd) cd.textContent = o.cerr ? '' : '· ' + dur(o.custom[1] - o.custom[0], isEN()); }
      if (path) path.outerHTML = pathField(pathOf(o));
      if (foot) patchFoot(foot, o);
    }
    function load() {
      var req = dataReq(o), key = req ? JSON.stringify(req) : null;
      if (key === o.key) return;
      o.key = key; o.loadErr = null; o.data = null; o.useRange = false; o.saveWanted = false;
      if (!req) return;
      var s = seed(o);
      if (s) { o.data = s; o.useRange = o.period === 'sel'; return; }
      window.API.call('snapshot', { req: req }).then(function (snap) {
        if (closed || o.key !== key) return;
        o.data = snap; o.useRange = !!req.range;
        addPiece(snap && snap.history);
        if (o.saveWanted) { o.saveWanted = false; save(); } else soft();
      }, function (err) {
        if (closed || o.key !== key) return;
        o.loadErr = err; o.saveWanted = false;
        soft();
      });
    }
    function setPeriod(k) {
      o.period = k;
      if (k === 'custom') o.cerr = checkCustom(o);
      load();
    }
    // Another zone: a time field that reads is written in it (the same moment), a text with a mistake stays as typed;
    // then the period is checked again in the new zone.
    function setTz(k) {
      if (o.tz === k) return;
      var was = o.tz;
      o.tz = k;
      o.text = o.text.map(function (s) { var ms = parseInput(s, was); return isNaN(ms) ? s : inputText(ms, k, isEN()); });
      o.cerr = checkCustom(o);
      load();
    }
    // A time field is done (Enter, Tab, a click elsewhere): a valid period is written back in the fields' format and its
    // data asked for; the rest follows in place.
    function commitCustom(input) {
      if (closed || painting) return;
      readField(input);
      clearTimeout(typeT);
      if (!o.cerr) {
        o.text = [inputText(o.custom[0], o.tz, isEN()), inputText(o.custom[1], o.tz, isEN())];
        [].forEach.call(dlg.querySelectorAll('input.rp-in'), function (x) { var v = o.text[x.dataset.act === 'cfrom' ? 0 : 1]; if (x.value !== v) x.value = v; });
        load();
      }
      soft();
    }
    function saveErr(e) { var x = errPairs(e); return ['Не удалось сохранить отчет. ' + x.title[0], 'Could not save the report. ' + x.title[1]]; }
    function save() {
      if (closed || o.saving) return;
      var r = periodRange(o), en = isEN();
      if (!r || fileCount(o).why || o.loadErr) return;
      if (!o.data) { o.saveWanted = true; soft(); return; } // load() saves as soon as the period's data is in
      var png = null;
      if (o.fmt.png) { try { png = drawPng(o); } catch (e) { console.error(e); } }
      var req = {
        rows: lines(o).map(function (t) { return t.id; }), fromMs: Math.round(r[0]), toMs: Math.round(r[1]),
        formats: FMT.map(function (f) { return f[0]; }).filter(function (k) { return o.fmt[k]; }), tzOffsetMin: offMin(r[1], o.tz),
        findings: o.findings ? findingsOf(o).map(function (x) { var s = x.f[en ? 1 : 0]; return o.from === 'trace' ? probeLabel(x.t) + ': ' + s : s; }) : [],
        pngBase64: png, dir: o.dir,
      };
      o.saving = true; o.err = null;
      soft();
      window.API.call('export_report', { req: req }).then(function (res) {
        if (closed) return;
        o.saving = false; o.res = res || { dir: '', files: [] }; o.what = savedWhat(o);
        paint();
        focusAct('open-dir');
      }, function (e) {
        if (closed) return;
        o.saving = false; o.err = saveErr(e);
        soft();
        focusAct('save');
      });
    }
    function pickDir() {
      window.API.call('pick_folder', {}).then(function (p) {
        if (closed) return;
        if (p) { o.dir = String(p); o.err = null; }
        soft();
        focusAct('dir');
      }, function (e) { if (closed) return; o.err = errPairs(e).title; soft(); });
    }
    // The core made the report folder itself (res.dir); «Open report» opens the report.html in it.
    function openPath(p) {
      var had = !!o.err;
      o.err = null;
      window.API.call('open_folder', { path: p }).then(function () { if (!closed && had) paint(); }, function (e) {
        if (closed) return;
        var x = errPairs(e);
        o.err = ['Не удалось открыть. ' + x.title[0], 'Could not open it. ' + x.title[1]];
        paint();
      });
    }
    function copyPath() {
      var text = String(o.res.dir || ''), done = function () {
        o.copied = true; paint(); focusAct('copy');
        setTimeout(function () { if (!closed && o.copied) { o.copied = false; paint(); } }, 1500);
      };
      var fallback = function () {
        var ta = document.createElement('textarea');
        ta.value = text; ta.style.cssText = 'position:fixed;left:-9999px;top:0';
        document.body.appendChild(ta); ta.select();
        try { document.execCommand('copy'); done(); } catch (e) { /* nothing to copy with */ }
        ta.remove();
      };
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(done, fallback); else fallback();
    }
    function close(quiet) {
      if (closed) return;
      closed = true;
      clearTimeout(typeT);
      window.removeEventListener('keydown', onKey, true);
      host.hidden = true; host.innerHTML = '';
      if (host._rp === ctl) host._rp = null;
      if (quiet === true) return;
      if (prev && prev.isConnected && prev.focus) prev.focus({ preventScroll: true });
      if (opts.onClose) opts.onClose(o.res || null);
    }
    function focusables() {
      return [].slice.call(dlg.querySelectorAll('button:not([disabled]), input, [tabindex="0"]')).filter(function (x) { return x.offsetParent !== null; });
    }
    // Keys while the dialog is open, for the whole page and before the screen's own handlers: Esc closes the time zone
    // list or the dialog, Tab stays in the dialog (also when the focus fell out of it onto the page).
    function onKey(e) {
      if (closed || e.isComposing) return;
      if (e.key === 'Escape') {
        e.preventDefault(); e.stopPropagation();
        if (o.tzMenu) { o.tzMenu = false; paint(); focusAct('tz'); } else close();
        return;
      }
      if (e.key !== 'Tab') return;
      var list = focusables(), ae = document.activeElement, first = list[0], last = list[list.length - 1];
      if (!list.length) return;
      if (!dlg.contains(ae)) { e.preventDefault(); (e.shiftKey ? last : first).focus(); }
      else if (e.shiftKey && ae === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && ae === last) { e.preventDefault(); first.focus(); }
    }

    root.addEventListener('click', function (e) {
      var t = e.target;
      e.stopPropagation();
      if (t === root || t === dlg || t.classList.contains('ms-scrim')) { close(); return; }
      var b = t.closest('[data-act]');
      if (!b || !dlg.contains(b)) { if (o.tzMenu && !t.closest('.rp-pmenu')) { o.tzMenu = false; paint(); } return; }
      var act = b.dataset.act, k = b.dataset.key;
      if (act === 'cfrom' || act === 'cto') return;
      var menuWas = o.tzMenu;
      if (act !== 'tz' && act !== 'tz-pick') o.tzMenu = false;
      if (act === 'close') { close(); return; }
      if (act === 'save') { if (!b.disabled) save(); return; }
      if (act === 'dir') { pickDir(); return; }
      if (act === 'open-dir') { openPath(o.res.dir); return; }
      if (act === 'open-file') { var dir = String(o.res.dir || ''); openPath(dir + (dir.indexOf('\\') >= 0 ? '\\' : '/') + 'report.html'); return; }
      if (act === 'copy') { copyPath(); return; }
      if (act === 'fmt') o.fmt[k] = !o.fmt[k];
      else if (act === 'tg' || act === 'pr') o.off[k] = !o.off[k];
      else if (act === 'tg-all') { var all = WT.targets.every(function (x) { return !o.off[x.id]; }); WT.targets.forEach(function (x) { o.off[x.id] = all; }); }
      else if (act === 'pin') o.off.pinned = !o.off.pinned;
      else if (act === 'fnd') o.findings = !o.findings;
      else if (act === 'period') { if (b.getAttribute('aria-disabled') === 'true') { if (menuWas) paint(); return; } setPeriod(k); }
      else if (act === 'tz') o.tzMenu = !o.tzMenu;
      else if (act === 'tz-pick') { setTz(k); o.tzMenu = false; paint(); focusAct('tz'); return; }
      else { if (menuWas) paint(); return; }
      o.err = null;
      paint();
      if (act === 'period' && k === 'custom') focusAct('cfrom');
    });
    // Typing a time: the numbers follow a moment after a valid period is typed.
    root.addEventListener('input', function (e) {
      if (!isTime(e.target)) return;
      readField(e.target);
      clearTimeout(typeT);
      if (!o.cerr) typeT = setTimeout(function () { if (!closed && !o.cerr) { load(); soft(); } }, 400);
      soft();
    });
    root.addEventListener('change', function (e) { if (isTime(e.target)) commitCustom(e.target); });
    root.addEventListener('keydown', function (e) {
      e.stopPropagation();
      if (e.key === 'Enter' && isTime(e.target)) { e.preventDefault(); commitCustom(e.target); return; }
      var cb = e.target.closest && e.target.closest('[role="checkbox"]');
      if (cb && (e.key === ' ' || e.key === 'Enter')) { e.preventDefault(); cb.click(); }
    });
    window.addEventListener('keydown', onKey, true);

    load();
    host.hidden = false;
    paint();
    var x = dlg.querySelector('.rp-head [data-act="close"]');
    if (x) x.focus({ preventScroll: true });
    return ctl;
  }

  /* ---------- refresh ---------- */
  function refreshAll() {
    STRIPS = STRIPS.filter(function (s) { return s.el._he === s; });
    BARS = BARS.filter(function (b) { return b.el._he === b; });
    STRIPS.forEach(function (s) { s.render(); });
    BARS.forEach(function (b) { b.render(); });
    fixNarrow(document);
  }
  // The narrow frame and the readout depend on the width in pixels: redo them on any change of the page width.
  var lastW = 0;
  function onWidth() { var w = document.documentElement.clientWidth; if (w !== lastW) { lastW = w; refreshAll(); } }
  window.addEventListener('resize', onWidth);
  if (window.ResizeObserver) new ResizeObserver(onWidth).observe(document.documentElement);

  window.HE = {
    history: history, pastBar: pastBar, empty: empty, report: report,
    errorInfo: errorInfo, rowError: rowError, refresh: refreshAll, L: L,
  };
})();
