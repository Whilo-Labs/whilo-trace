// Live: what the screen is looking at (window, live or past, selected period, traced target)
// and the loop that keeps WT fresh. Every change asks the core for a new snapshot (API "snapshot"),
// loads it with WT.apply and tells the screen through Live.onData. Round events refresh the data
// at most once a second and only while the screen shows live data.
// A command that changes targets or settings (add_target, set_paused, ...) refreshes the data too, so a part
// only calls the command. One snapshot is on its way at a time: a refresh asked after a change it does not
// cover (Live.set, a command, a round) gets one more snapshot after it, a refresh it covers gets the same one.
// A snapshot that does not come (the core is not up or went away) goes to Live.onError; Live.failing stays
// true until the next snapshot is in. The screen decides what to show and when to ask again.
(function () {
  var WT = window.WT, API = window.API;
  var state = { window: (WT.settings && WT.settings.window) || '10m', endMs: null, range: null, trace: null, buckets: 120 };
  var subs = [], errSubs = [], pending = null, pendingGen = -1, queued = null, last = 0, timer = null, gen = 0;
  // The saved window applies once: from get_settings, or from the first snapshot when the core came up later.
  var settingsSeen = false, windowTouched = false;
  var CHANGES = { add_target: 1, remove_target: 1, set_paused: 1, set_interval: 1, pin_hop: 1, unpin_hop: 1, set_settings: 1 };
  var READS = { snapshot: 1, get_settings: 1, recent_inputs: 1, pick_folder: 1, open_folder: 1, export_report: 1 };
  function noop() {}

  // The trace screen opens on trace.html?id=<row id>: its first snapshot already carries the trace.
  if (/trace\.html$/.test(location.pathname)) state.trace = new URLSearchParams(location.search).get('id') || null;
  // Parts that read time labels while loading get labels for now until the first snapshot is in.
  if (!WT.ticks || !WT.ticks['10m']) WT.ticks = WT.ticksAt(Date.now());

  function request() {
    return { window: state.window, endMs: state.endMs, range: state.range, trace: state.trace, buckets: state.buckets };
  }
  function emit() { subs.forEach(function (cb) { try { cb(WT); } catch (e) { console.error(e); } }); }

  function fetchOnce() {
    var req = request();
    pendingGen = gen;
    pending = API.call('snapshot', { req: req }).then(function (snap) {
      WT.apply(snap);
      window.Live.shown = req;
      window.Live.failing = false;
      last = Date.now();
      pending = null;
      if (window.WTTheme) window.WTTheme.sync(WT.settings);
      if (!settingsSeen) {
        settingsSeen = true;
        var w = snap.settings && snap.settings.window;
        if (!windowTouched && w && WT.windowSeconds[w] && w !== state.window) {
          state.window = w;
          gen++;
          refresh().catch(noop);
        }
      }
      emit();
      return WT;
    }, function (err) {
      pending = null;
      window.Live.failing = true;
      console.error('snapshot failed', err);
      errSubs.forEach(function (cb) { try { cb(err); } catch (e) { console.error(e); } });
      throw err;
    });
    return pending;
  }
  function refresh() {
    if (!pending) return fetchOnce();
    if (pendingGen === gen) return pending; // the snapshot on its way already covers every change
    if (!queued) {
      queued = pending.catch(noop).then(function () {
        queued = null;
        return pending || fetchOnce();
      });
    }
    return queued;
  }

  function onRound() {
    gen++;
    if (state.endMs) return; // looking at the past: live rounds do not move the screen
    var wait = Math.max(0, 1000 - (Date.now() - last));
    if (timer) return;
    timer = setTimeout(function () { timer = null; refresh().catch(noop); }, wait); // a failure goes to onError
  }

  // A command that changes something marks the data as old; one that changes what the snapshot shows refreshes it.
  var call = API.call;
  API.call = function (cmd) {
    var p = call.apply(API, arguments);
    if (!READS[cmd]) p.then(function () { gen++; if (CHANGES[cmd]) refresh().catch(noop); }, noop); // the caller shows the error
    return p;
  };

  window.Live = {
    state: state,
    // The request that the data in WT answers (state may already be ahead of it).
    shown: null,
    // Merge a change into the state ({window}, {endMs}, {range: [fromMs, toMs) | null}, {trace}) and refresh.
    set: function (patch) { if ('window' in patch) windowTouched = true; Object.assign(state, patch); gen++; return refresh(); },
    refresh: refresh,
    onData: function (cb) { subs.push(cb); },
    // cb(err) when a snapshot did not come; the data in WT stays as it was.
    onError: function (cb) { errSubs.push(cb); },
    // The last snapshot asked for did not come.
    failing: false,
    ready: null,
  };

  API.on('round', onRound);
  // Saved settings first: the first snapshot is for the saved window, theme and language apply before the screen mounts.
  window.Live.ready = API.call('get_settings').then(function (s) {
    if (s) {
      settingsSeen = true;
      WT.settings = Object.assign({}, WT.settings, s);
      if (s.window && WT.windowSeconds[s.window]) state.window = s.window;
      if (window.WTTheme) window.WTTheme.sync(WT.settings);
    }
  }, function (err) { console.error('get_settings failed', err); }).then(refresh);
  // A failed first snapshot is logged and goes to onError; screens that load later still get it from ready.
  window.Live.ready.catch(noop);
})();
