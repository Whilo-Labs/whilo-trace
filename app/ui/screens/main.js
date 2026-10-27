// Whilo Trace, the «All targets» screen: the parts put together and linked.
// Header (WTH, parts/header), summary (SUM, parts/summary), timeline stack (WTT, parts/timeline),
// history strip, past bar, first run, error texts and the report dialog (HE, parts/history).
// The parts draw, update themselves in place and handle their own controls. This file lays them out,
// links them through their callbacks (one order for the summary and the stack, the window, the picked period,
// the past, the trace) and talks to the core through API and Live. It mounts the screen once the first
// snapshot is in; until then, or when no snapshot comes, a calm message stands in the data area and the request
// goes again. On new data (Live.onData) it feeds the stack and redraws the status line; the header, the summary,
// the strip and the past bar follow Live.onData themselves. Error texts come from HE.errorInfo.
(function () {
  'use strict';
  var WT = window.WT, Live = window.Live, API = window.API;
  var EN = document.documentElement.dataset.lang === 'en';
  var HOLD_MS = 5000;   // how long a new order waits for the data to confirm it (the summary and the stack do the same)
  var RETRY_MS = 3000;  // a snapshot that did not come is asked again after this
  var SLOW_MS = 5000;   // the first snapshot still not in: the calm message instead of an empty window
  var NOTE_MS = 8000;   // how long the line about a failed action stays in the status line
  function noop() {}
  // One broken part must not take the whole screen down: log it and go on with the rest.
  function safe(fn) { try { return fn(); } catch (err) { console.error(err); } }

  /* ---------- text ---------- */
  function L(ru, en) { return EN ? en : ru; }
  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  function T(ru, en, cls) { return '<span' + (cls ? ' class="' + cls + '"' : '') + ' data-en="' + esc(en) + '">' + esc(L(ru, en)) + '</span>'; }
  function $(s, r) { return (r || document).querySelector(s); }
  function $$(s, r) { return [].slice.call((r || document).querySelectorAll(s)); }
  function cap(s) { return s ? s.charAt(0).toUpperCase() + s.slice(1) : s; }
  function plural(n, one, few, many) {
    var a = n % 10, b = n % 100;
    return a === 1 && b !== 11 ? one : a >= 2 && a <= 4 && (b < 12 || b > 14) ? few : many;
  }
  function probeLabel(p) { return WT.probes[p] ? WT.probes[p].label : String(p || '').toUpperCase(); }
  // Links to the trace keep only the check flags of the address (theme, language, mock).
  function keep(extra) {
    var q = new URLSearchParams(location.search), p = new URLSearchParams();
    ['theme', 'lang', 'mock'].forEach(function (k) { if (q.get(k)) p.set(k, q.get(k)); });
    Object.keys(extra || {}).forEach(function (k) { if (extra[k] != null) p.set(k, extra[k]); });
    var s = p.toString();
    return s ? '?' + s : '';
  }

  /* ---------- error texts: one place for every part (HE.errorInfo) ---------- */
  // Ours stand in only while the history part has no text for a code (the same words). [ru, en]: status, text.
  var OWN_ERR = {
    core_unavailable: [['нет данных', 'No data'], ['Данные пока не пришли. Whilo Trace пробует снова.', 'The data has not come in yet. Whilo Trace keeps trying.']],
    no_such_target: [['цели уже нет', 'Target is gone'], ['Этой цели уже нет.', 'This target is gone.']],
    no_such_hop: [['хопа нет в трассе', 'Hop not in the trace'], ['Этого хопа нет в трассе или у него нет адреса.', 'This hop is not in the trace or has no address.']],
    not_implemented: [['не работает в этой сборке', 'Not in this build'], ['Это пока не работает в этой сборке.', 'This does not work in this build yet.']],
    db: [['ошибка истории', 'History error'], ['Не удалось прочитать или записать историю.', 'Could not read or write the history.']],
    io: [['системная ошибка', 'System error'], ['Системная ошибка.', 'System error.']],
  };
  function inLang(v) { return Array.isArray(v) ? v[EN ? 1 : 0] || '' : typeof v === 'string' ? v : ''; }
  // { code, status, text } in the interface language. en: the English of the same (for data-en); the history part
  // gives the interface language only, so a Russian screen keeps our English there.
  function errInfo(err, ctx) {
    var raw = String(err == null ? '' : err && err.message ? err.message : err), code = raw.split(':')[0].trim();
    var own = OWN_ERR[code], e = null;
    if (window.HE && typeof HE.errorInfo === 'function') { try { e = HE.errorInfo(raw, ctx || {}); } catch (x) { console.error(x); } }
    var status = (e && inLang(e.status)) || (own ? L(own[0][0], own[0][1]) : '');
    var text = (e && inLang(e.text)) || (own ? L(own[1][0], own[1][1]) : '');
    return {
      code: code, status: status, text: text,
      en: { status: EN ? status : own ? own[0][1] : '', text: EN ? text : own ? own[1][1] : '' },
    };
  }

  /* ---------- rows, items and the one order ---------- */
  // An item is one line of the summary and one card of the stack: a target with all its probe types,
  // or a pinned hop. The order is the core's (rows come in the user's order). After a drag in either list the
  // new order shows in both at once and holds until a snapshot comes in that order or HOLD_MS pass.
  var BY = {}, ITEMS = [], ITEM = {}, PEND = null;
  function itemKey(t) { return t.hopOf ? 'h:' + t.id : 't:' + (t.targetId != null ? t.targetId : t.host); }
  function held(items) {
    if (!PEND) return items;
    var flat = [], here = {}, pos = {};
    items.forEach(function (it) { flat = flat.concat(it.lines); });
    flat.forEach(function (id) { here[id] = true; });
    var want = PEND.ids.filter(function (id) { return here[id]; });
    want.forEach(function (id, i) { pos[id] = i; });
    if (Date.now() > PEND.until || flat.filter(function (id) { return id in pos; }).join() === want.join()) { PEND = null; return items; }
    return items.map(function (it, i) {
      var r = Infinity;
      it.lines.forEach(function (id) { if (id in pos) r = Math.min(r, pos[id]); });
      return { it: it, r: r, i: i };
    }).sort(function (a, b) { return a.r - b.r || a.i - b.i; }).map(function (x) { return x.it; });
  }
  function syncItems() {
    var by = {}, list = [];
    BY = {};
    WT.targets.forEach(function (t) {
      BY[t.id] = t;
      var k = itemKey(t);
      if (!by[k]) { by[k] = { key: k, id: t.id, lines: [] }; list.push(by[k]); }
      by[k].lines.push(t.id);
    });
    ITEMS = held(list);
    ITEM = {};
    ITEMS.forEach(function (it) { ITEM[it.id] = it; });
  }
  function linesOf(id) { return ITEM[id] ? ITEM[id].lines : [id]; }
  function flatIds() { var out = []; ITEMS.forEach(function (it) { out = out.concat(it.lines); }); return out; }
  function counts() {
    var tg = 0, hp = 0;
    ITEMS.forEach(function (it) { if (BY[it.id].hopOf) hp++; else tg++; });
    return {
      tg: tg, hp: hp,
      ru: tg + ' ' + plural(tg, 'цель', 'цели', 'целей') + (hp ? ', ' + hp + ' ' + plural(hp, 'хоп', 'хопа', 'хопов') : ''),
      en: tg + ' target' + (tg === 1 ? '' : 's') + (hp ? ', ' + hp + ' hop' + (hp === 1 ? '' : 's') : ''),
    };
  }
  // A new order from the summary or the stack: the other list takes it, the core saves it (set_order),
  // fresh data confirms it.
  function setOrder(ids, from) {
    PEND = { ids: ids.slice(), until: Date.now() + HOLD_MS };
    syncItems();
    if (from !== 'stack') safe(renderStack);
    if (from !== 'summary' && sum) safe(function () { sum.setOrder(ids); });
    safe(renderStatus);
    // a snapshot that does not come after it is Live's (onError), not a failed order
    API.call('set_order', { ids: ids }).then(function () { Live.refresh().catch(noop); }, function (err) { failed('set_order', err); });
  }

  /* ---------- screen state ---------- */
  var WIN = Live.state.window;   // window asked for (the header shows it at once)
  var SEL = null;                // item picked in the summary: its timeline is highlighted
  var RANGE = null;              // period picked on a timeline, [fromMs, toMs), or null
  var view = null, sum = null, firstEl = null, downEl = null;
  var booted = false, hdrOn = false;

  var app = $('#app'), bodyEl = $('#body'), scrollEl = $('#scroll'), sumEl = $('#sum'), stackEl = $('#stack');
  var pastEl = $('#past'), histEl = $('#hist'), statusEl = $('#status'), modal = $('#modal');

  // Window of the data on screen; the asked one may still be on its way.
  function shownWin() { return (Live.shown && Live.shown.window) || WIN; }
  function sameRange(a, b) { return a === b || (!!a && !!b && a[0] === b[0] && a[1] === b[1]); }
  // The period picked on a timeline goes to Live: the core sends rangeStats for it, the summary and the
  // stack show them. A new window or the past drops it.
  function pickRange(ms) {
    var next = ms ? [ms[0], ms[1]] : null;
    if (sameRange(next, RANGE)) return;
    RANGE = next;
    Live.set({ range: RANGE }).catch(noop);
  }
  function dropRange() {
    RANGE = null;
    if (view && view.rangeMs) safe(function () { view.setRangeMs(null); });
  }

  /* ---------- header and settings (6B: a card on the right, the data area narrows) ---------- */
  function mountHeader() {
    if (hdrOn) return;
    hdrOn = true;
    var cb = { onWindow: setWindow, onSettings: onSettings, onInterval: onInterval, focus: booted && !ITEMS.length };
    $('#hdr').outerHTML = WTH.header(cb);
    bodyEl.insertAdjacentHTML('beforeend', WTH.settingsPanel('side', true));
    WTH.bindAll(cb);
  }
  // The window from the header: saved in the core and asked from Live; the strip frame follows the data.
  function setWindow(key) {
    if (!WT.windowSeconds[key]) return;
    WIN = key;
    dropRange();
    API.call('set_settings', { settings: { window: key } }).catch(function (err) { console.error('set_settings failed', err); });
    Live.set({ window: key, range: null }).catch(noop);
  }
  // Settings saved from the card (the header part saves them): new latency thresholds repaint the charts.
  function onSettings() {
    safe(renderStack);
    if (sum) safe(function () { sum.update(); });
  }
  // The interval for a new target changed in the header: the first-run message says the new one.
  function onInterval() { if (firstEl) safe(paintFirst); }

  /* ---------- summary ---------- */
  function mountSummary() {
    sum = SUM.mount(sumEl, {
      onTrace: function (rowId) { openTrace(rowId); },
      onSelect: function (itemId) { SEL = itemId; markPicked(); },
      onOrder: function (ids) { setOrder(ids, 'summary'); },
      onReport: openReport,
      onError: function (code, cmd, rowId) { failed(cmd, code, rowId); },
      // figures for a picked period once the data on screen is for it (rangeStats)
      range: function () { return Live.shown ? Live.shown.range : null; },
    });
  }

  /* ---------- timeline stack ---------- */
  function stackItems() { return WTT.fromTargets(flatIds().map(function (id) { return BY[id]; })); }
  function unpin(rowId) { API.call('unpin_hop', { id: rowId }).catch(function (err) { failed('unpin_hop', err, rowId); }); }
  function renderStack() {
    if (!ITEMS.length) { stackEl.hidden = true; return; }
    stackEl.hidden = false;
    if (!view) {
      view = WTT.view(stackEl, stackItems(), {
        stack: true, select: true, reorder: true, layout: 'side', h: 64, hj: 12, win: false, mode: 'tracks',
        window: shownWin(), rangeMs: RANGE,
        onRange: function (ms) { pickRange(ms); },
        onOrder: function (ids) { setOrder(ids, 'stack'); },
        onUnpin: function (rowId) { unpin(rowId); },
        onError: function (code, cmd, rowId) { failed(cmd, code, rowId); },
      });
      // The part also redraws after its own actions (picked period, probe switch, drop): decorate after it.
      var draw = view.render;
      view.render = function () { draw.apply(view, arguments); decorateStack(); };
      decorateStack();
    } else view.update(stackItems(), { window: shownWin() });
    fixAxis();
  }
  function markPicked() { $$('.tl[data-id]', stackEl).forEach(function (c) { c.classList.toggle('scr-picked', c.dataset.id === SEL); }); }
  // The item picked in the summary is highlighted in the stack. The «paused» chip of a paused track is drawn by
  // the timeline part (.tl-pz); a part without it gets ours next to the track status.
  function decorateStack() {
    markPicked();
    $$('.tl[data-id]', stackEl).forEach(function (c) {
      linesOf(c.dataset.id).forEach(function (lid, k) {
        var t = BY[lid], lab = trackLabel(c, k), st = lab && $('.tl-status', lab), chip = lab && $('.scr-pz', lab);
        var own = lab && $('.tl-pz', lab), want = !!(t && t.paused && st && !own);
        if (want && !chip) st.insertAdjacentHTML('afterend', '<span class="chip scr-pz">' + T('пауза', 'paused') + '</span>');
        else if (chip && !want) chip.remove();
      });
    });
  }
  // Label of track k in a card: the row on the left of the track.
  function trackLabel(c, k) {
    var side = c.querySelectorAll('.tl-srow');
    if (side.length) return side[k] ? $('.tl-l3', side[k]) : null;
    var th = c.querySelectorAll('.tl-thead');
    return th.length ? th[k] || null : $('.tl-id', c);
  }
  // The shared time axis sticks to the bottom of the data area only when a whole timeline is visible
  // (it then lies on the lower edge of that card, as on the approved screen). Otherwise (only the top of the
  // stack is visible) it stays under the stack and does not cover the head of the first card.
  // Checked again on scroll and whenever the layout moves (fonts, the strip, the past bar, new rows).
  function fixAxis() {
    var ax = $('.tl-axisbar', stackEl), first = $('.tl-stack > .tl', stackEl);
    if (!ax || !first || stackEl.hidden) return;
    var free = first.getBoundingClientRect().bottom > scrollEl.getBoundingClientRect().bottom + 0.5;
    if (ax.classList.contains('scr-free') !== free) ax.classList.toggle('scr-free', free);
  }
  scrollEl.addEventListener('scroll', fixAxis, { passive: true });
  window.addEventListener('resize', fixAxis);
  if (window.ResizeObserver) { var ro = new ResizeObserver(function () { fixAxis(); }); [scrollEl, sumEl, stackEl].forEach(function (el) { ro.observe(el); }); }
  // A double click on a timeline label opens the trace, as a double click on a summary row.
  app.addEventListener('dblclick', function (e) {
    var lab = e.target.closest('#stack .tl-side, #stack .tl-head');
    if (!lab || e.target.closest('button')) return;
    var c = lab.closest('.tl[data-id]');
    if (c) openTrace(c.dataset.id);
  });

  /* ---------- history strip, past bar, report dialog ---------- */
  // The frame moved on the strip: the past (Live endMs) or back to now (null); a picked period goes away.
  function mountHistory() {
    HE.history(histEl, {
      onEnd: function (endMs) { dropRange(); return Live.set({ endMs: endMs, range: null }); },
    });
    HE.pastBar(pastEl, {});
  }
  // The dialog gets what is on screen: the window or the past frame, and the picked period.
  // While it is open it takes Esc and Tab for the whole page itself (HE.report).
  function openReport() { HE.report(modal, { from: 'main', range: RANGE }); }

  /* ---------- trace ---------- */
  // A target opens its trace; a pinned hop opens the trace of its target on that hop.
  function openTrace(id) {
    var t = BY[id];
    if (!t) return;
    location.href = 'trace.html' + keep(t.hopOf ? { id: t.hopOf, hop: t.hop } : { id: id });
  }

  /* ---------- first run: one message from the history part (8A); the field is in the header ---------- */
  // Interval for a new target, as picked in the header.
  function headerInterval() {
    var v = window.WTH && typeof WTH.interval === 'function' ? +WTH.interval() : NaN;
    if (v > 0) return v;
    var on = $('.wth-intervals [data-iv][aria-selected="true"]');
    return on ? parseFloat(on.getAttribute('data-iv')) : WT.intervals[2];
  }
  // A quick-start button adds the target the way «Добавить» in the header does (its probe types and interval);
  // a header without WTH.add leaves the adding to the history part.
  function firstOpts() {
    var o = { interval: headerInterval() };
    if (window.WTH && typeof WTH.add === 'function') {
      o.onPick = function (host) {
        return Promise.resolve(WTH.add(host)).catch(function (err) { console.error('add_target failed', err); }); // the header shows it
      };
    }
    return o;
  }
  function paintFirst() {
    var a = document.activeElement, host = firstEl.contains(a) && a.dataset ? a.dataset.host : null;
    HE.empty(firstEl, firstOpts());
    if (host) { var b = $('[data-host="' + host + '"]', firstEl); if (b) b.focus({ preventScroll: true }); }
  }
  function showFirst() {
    stackEl.hidden = true;
    if (firstEl) return;
    firstEl = document.createElement('div');
    firstEl.className = 'scr-first';
    scrollEl.appendChild(firstEl);
    safe(paintFirst);
  }
  // The first target is in: the message goes; the focus it held (a quick-start button) moves to the field.
  function hideFirst() {
    if (!firstEl) return;
    var a = document.activeElement, lost = !a || a === document.body || firstEl.contains(a);
    firstEl.remove();
    firstEl = null;
    if (lost && window.WTH && WTH.focusField) safe(WTH.focusField);
  }

  /* ---------- the core does not answer ---------- */
  // No first snapshot (the core is not up, or it does not answer): a calm message in the data area instead of
  // the first run or an empty window; the strip and the status line wait. Later, with data on screen, the data
  // stays and the status line says it is not fresh. Either way the snapshot is asked again every RETRY_MS.
  var retryT = 0, slowT = 0;
  function retry() {
    if (retryT) return;
    retryT = setTimeout(function () { retryT = 0; Live.refresh().catch(retry); }, RETRY_MS);
  }
  function showDown() {
    hideFirst();
    stackEl.hidden = true;
    histEl.hidden = statusEl.hidden = true;
    var e = errInfo('core_unavailable'), html = '<div class="scr-down" role="status">' +
      '<h2 data-en="' + esc(cap(e.en.status)) + '">' + esc(cap(e.status)) + '</h2>' +
      '<p data-en="' + esc(e.en.text) + '">' + esc(e.text) + '</p></div>';
    if (!downEl) { downEl = document.createElement('div'); scrollEl.appendChild(downEl); }
    if (downEl.innerHTML !== html) downEl.innerHTML = html;
  }
  function hideDown() {
    histEl.hidden = statusEl.hidden = false;
    if (downEl) { downEl.remove(); downEl = null; }
  }

  /* ---------- status line ---------- */
  // A failed action (pause, removal, unpin, order) or data that no longer comes: a short neutral line.
  var NOTE = null, noteT = 0;
  var DID = {
    pause: ['Не удалось поставить на паузу', 'Could not pause'],
    resume: ['Не удалось продолжить опрос', 'Could not resume probing'],
    remove_target: ['Не удалось убрать цель', 'Could not remove the target'],
    unpin_hop: ['Не удалось убрать хоп с главного экрана', 'Could not unpin the hop from the main screen'],
    set_order: ['Не удалось сохранить порядок', 'Could not save the order'],
  };
  function failed(cmd, err, rowId) {
    console.error(cmd + ' failed', err);
    var t = BY[rowId], key = cmd === 'set_paused' ? (t && t.paused ? 'resume' : 'pause') : cmd;
    var did = DID[key] || ['Не удалось выполнить действие', 'The action failed'];
    var e = errInfo(err, t ? { host: t.host, probe: t.probe, interval: t.interval } : null);
    NOTE = { ru: did[0] + '.' + (e.text && !EN ? ' ' + e.text : ''), en: did[1] + '.' + (e.en.text ? ' ' + e.en.text : '') };
    clearTimeout(noteT);
    noteT = setTimeout(function () { NOTE = null; safe(renderStatus); }, NOTE_MS);
    safe(renderStatus);
  }
  function noteHtml() {
    var n = NOTE;
    if (Live.failing) { var e = errInfo('core_unavailable'); n = { ru: e.text, en: e.en.text }; }
    if (!n) return '';
    var shown = L(n.ru, n.en);
    return '<span class="wts-item scr-note" role="status" title="' + esc(shown) + '" data-en="' + esc(n.en) + '">' + esc(shown) + '</span>';
  }
  function statusHtml() {
    var c = counts(), by = {}, order = [];
    ITEMS.forEach(function (it) {
      it.lines.forEach(function (id) {
        var t = BY[id];
        if (!t || t.hopOf) return;
        var lb = probeLabel(t.probe);
        if (!(lb in by)) { by[lb] = 0; order.push(lb); }
        by[lb]++;
      });
    });
    var pps = WT.ratePps || 0, shown = pps ? WT.fmt(pps) : '0'; // as WT.fmt: whole from 10, one decimal below
    var ppsRu = shown + ' ' + (shown.indexOf(',') < 0 ? plural(parseInt(shown, 10), 'пакет', 'пакета', 'пакетов') : 'пакета') + '/с';
    var ppsEn = (pps >= 10 || !pps ? String(Math.round(pps)) : pps.toFixed(1)) + ' packets/s';
    var left = c.tg || c.hp ? T(c.ru + ' на экране', c.en + ' on screen') : T('Целей пока нет', 'No targets yet');
    var probes = order.length ? '<span class="wts-item scr-probes">' + T('пробы', 'probes') + order.map(function (lb) {
      return '<span class="chip">' + T(lb, lb) + '</span>' + T(String(by[lb]), String(by[lb]), 'num n');
    }).join('') + '</span>' : '';
    return '<footer class="wts"><span class="wts-item num">' + left + '</span>' + noteHtml() + probes +
      '<span class="wts-item wts-right num has-tip" tabindex="0">' + T(ppsRu, ppsEn) +
      '<span class="wth-tip to-right up" role="tooltip">' + T('Сколько пакетов в секунду отправляют пробы всех целей: по пакету на хоп за раунд. Чем больше типов проб и короче интервал, тем чаще узлы ограничивают свои ответы.',
        'Packets per second sent by the probes of all targets: one packet per hop per round. More probe types and a shorter interval make nodes limit their replies more often.') + '</span></span></footer>';
  }
  // Patched in place (not rewritten), so the hint under the pointer and the focus stay.
  function patch(to, from) {
    var i, a, x = to.firstChild, y = from.firstChild;
    if (to.nodeType === 1) {
      for (i = to.attributes.length - 1; i >= 0; i--) { a = to.attributes[i]; if (!from.hasAttribute(a.name)) to.removeAttribute(a.name); }
      for (i = 0; i < from.attributes.length; i++) { a = from.attributes[i]; if (to.getAttribute(a.name) !== a.value) to.setAttribute(a.name, a.value); }
    } else if (to.nodeValue !== from.nodeValue) to.nodeValue = from.nodeValue;
    while (y) {
      var ny = y.nextSibling;
      if (!x) to.appendChild(y);
      else if (x.nodeType === y.nodeType && x.nodeName === y.nodeName) { patch(x, y); x = x.nextSibling; }
      else { var nx = x.nextSibling; to.replaceChild(y, x); x = nx; }
      y = ny;
    }
    while (x) { var rest = x.nextSibling; to.removeChild(x); x = rest; }
  }
  function renderStatus() {
    if (!booted) return;
    var fresh = statusEl.cloneNode(false);
    fresh.innerHTML = statusHtml();
    patch(statusEl, fresh);
  }

  /* ---------- live updates ---------- */
  function update() {
    syncItems();
    if (SEL && !ITEM[SEL]) SEL = null;
    if (!ITEMS.length) safe(showFirst);
    else { hideFirst(); safe(renderStack); }
    safe(renderStatus);
  }

  /* ---------- mount ---------- */
  // The first snapshot is in (at once or after the core came back): the whole screen mounts.
  function boot() {
    booted = true;
    clearTimeout(slowT);
    hideDown();
    WIN = Live.state.window;
    syncItems();
    var a = document.activeElement, idle = !a || a === document.body;
    safe(mountHeader);
    // a header that came up with the message of a silent core: the first run puts the caret in the field, as at once
    if (!ITEMS.length && idle && window.WTH && WTH.focusField) safe(WTH.focusField);
    safe(mountSummary);
    safe(mountHistory);
    update();
  }
  // No data: the header comes up (its controls work without data), the data area says what is going on.
  function down() {
    if (booted) { safe(renderStatus); retry(); return; }
    safe(mountHeader);
    safe(showDown);
    retry();
  }
  // The first snapshot may be in before this script runs (Live.ready keeps it), or come later (Live.onData).
  Live.onData(function () {
    if (retryT) { clearTimeout(retryT); retryT = 0; }
    if (booted) update(); else boot();
  });
  Live.onError(function () { down(); });
  Live.ready.then(function () { if (!booted) boot(); }, function (err) { console.error('first snapshot failed', err); down(); });
  slowT = setTimeout(function () { if (!booted) down(); }, SLOW_MS);
})();
