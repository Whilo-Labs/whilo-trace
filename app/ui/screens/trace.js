// Whilo Trace, screen «Трасса цели»: the trace part (TR) on live data, the report dialog of the history part (HE) over it.
// Address: trace.html?id=<row id>[&hop=<ttl>]. Without id the first target opens; a pinned hop row opens its target
// at that hop. ?mock=, ?theme= and ?lang= are kept on the way back to «Все цели».
// The part sends actions here; this file calls API and Live (docs/ui-contract.md) and feeds the part new data.
(function () {
  'use strict';
  var Q = new URLSearchParams(location.search);
  var el = document.getElementById('trace'), modal = document.getElementById('modal');
  var id = Q.get('id') || null, hop = parseInt(Q.get('hop'), 10), view = null, busy = {};
  var RETRY_MS = 3000;   // no snapshot: ask again this often (as the main screen)
  var FIRST_MS = 10000;  // the first snapshot has not come back by then: say so instead of loading on
  var NOTE_MS = 10000;   // how long the text of a failed action stays
  var down = false, retry = null, noteTimer = null, started = false, loaded = false;
  if (!(hop >= 1)) hop = null;
  function noop() {}

  function keep(extra) {
    var p = new URLSearchParams();
    ['theme', 'lang', 'mock'].forEach(function (k) { if (Q.get(k)) p.set(k, Q.get(k)); });
    Object.keys(extra || {}).forEach(function (k) { if (extra[k] != null) p.set(k, extra[k]); });
    var s = p.toString();
    return s ? '?' + s : '';
  }
  function byId(x) { return WT.targets.filter(function (t) { return t.id === x; })[0] || null; }
  // The address follows the shown target and hop, so a reload opens the same view.
  function setAddress() {
    try { history.replaceState(null, '', 'trace.html' + keep({ id: id, hop: hop })); } catch (e) { /* no history here */ }
  }

  // No snapshot from the core: the part says so calmly (over the data on screen, if there is any) and the
  // request repeats until data comes again (onData). Live logs the error itself.
  function lost() {
    if (!retry) retry = setInterval(function () { Live.refresh().catch(noop); }, RETRY_MS);
    if (!down) { down = true; if (view) view.update({ down: true }); }
  }
  function look(patch) { Live.set(patch).catch(lost); }

  // Commands that change data refresh the snapshot themselves (base/live.js). A failed one: its text from the
  // history part (HE.errorInfo, through TR.errorText) stands in the neutral line under the findings for a while.
  function clearNote() {
    clearTimeout(noteTimer); noteTimer = null;
    if (view && view.st.note) view.update({ note: null });
  }
  function act(cmd, args) {
    clearNote();
    return API.call(cmd, args).catch(function (e) {
      console.error(cmd, e);
      var t = byId(id);
      clearTimeout(noteTimer);
      noteTimer = setTimeout(clearNote, NOTE_MS);
      if (view) view.update({ note: TR.errorText(e, t ? { host: t.host, probe: t.probe, interval: t.interval } : {}) });
      throw e;
    });
  }
  // A hop counts as pinned when any probe row of this target pinned it (the part shows it the same way).
  function pinned(ttl) { return !!TR.pinsOf(id)[ttl]; }
  // «Вынести на главный экран» pins the hop; on a hop already pinned the same button takes that row off.
  // The hop stays busy until the data shows the change, so a second click does not pin it twice.
  function pin(ttl, pinnedId) {
    if (busy[ttl]) return;
    busy[ttl] = { want: !pinnedId, until: Date.now() + 3000 };
    act(pinnedId ? 'unpin_hop' : 'pin_hop', pinnedId ? { id: pinnedId } : { targetId: id, ttl: ttl })
      .catch(function () { delete busy[ttl]; });
  }

  // Which row: ?id=, a pinned hop row opens its target at that hop, without id the first target.
  function resolve() {
    var r = id ? byId(id) : null;
    if (r && r.hopOf) { if (hop == null) hop = r.hop; id = r.hopOf; }
    if (!id) { r = WT.targets.filter(function (t) { return !t.hopOf; })[0]; id = r ? r.id : null; }
  }
  // The view from what WT holds now. Before the first snapshot the trace asked for stays as it is.
  function sync() {
    var before = id;
    resolve();
    if (id !== before) { setAddress(); if (hop != null) view.update({ selected: hop }); }
    var r = id ? byId(id) : null, shown = Live.shown;
    if (shown) {
      // The core is asked for the trace of a known target only; an unknown id gets the calm card.
      var want = r ? id : null;
      if (Live.state.trace !== want) look({ trace: want });
      if (!r || shown.trace === id) loaded = true;
    }
    Object.keys(busy).forEach(function (k) { if (pinned(+k) === busy[k].want || Date.now() > busy[k].until) delete busy[k]; });
    view.update({ id: id, win: Live.state.window, loading: !loaded, dataRange: shown ? shown.range || null : null, down: down });
  }
  // New data: the core answers (again).
  function onData() {
    if (retry) { clearInterval(retry); retry = null; }
    down = false;
    sync();
  }

  function start() {
    if (started) return;
    started = true;
    resolve();
    view = TR.mount(el, {
      id: id, win: Live.state.window, selected: hop, loading: true, down: down,
      onBack: function () { location.href = 'index.html' + keep(); },
      onReport: openReport,
      onWindow: function (w) {
        look({ window: w });
        // Saved as on the main screen, so «Все цели» and the next trace open with it.
        API.call('set_settings', { settings: { window: w } }).catch(function (err) { console.error('set_settings failed', err); });
      },
      onInterval: function (v) { act('set_interval', { id: id, intervalS: v }).catch(noop); },
      onPause: function (paused) { act('set_paused', { id: id, paused: paused }).catch(noop); },
      onProbe: function (rowId) { id = rowId; setAddress(); view.update({ id: rowId }); look({ trace: rowId }); },
      onPin: pin,
      onRange: function (range) { look({ range: range }); },
      onSelect: function (ttl) { hop = ttl; setAddress(); },
    });
    if (id !== Q.get('id')) setAddress();
    Live.onData(onData);
    sync();
  }
  Live.ready.then(start, function () { start(); lost(); });
  // The first snapshot may not come back at all: the calm message instead of an endless loading screen.
  setTimeout(function () { if (!Live.shown) { start(); lost(); } }, FIRST_MS);

  /* ---------- report dialog over the screen ---------- */
  // «Отчет для провайдера»: the dialog of the history part (HE.report) shows itself in the modal box, saves the
  // report and closes on its own; it gets the target row and the period picked on the hop timeline ([fromMs, toMs)).
  function openReport() {
    if (!window.HE || !HE.report || !view) return;
    HE.report(modal, { from: 'trace', target: id, range: view.st.range || null });
  }
  // Esc while the dialog is open is the dialog's: HE.report listens for it on the document while it is open.
  // Fallback for a dialog that does not: Esc nobody took (the focus outside the dialog) closes it here.
  window.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape' || e.defaultPrevented || modal.hidden) return;
    e.preventDefault();
    if (modal._rp && modal._rp.close) modal._rp.close(); else { modal.hidden = true; modal.innerHTML = ''; }
  });
})();
