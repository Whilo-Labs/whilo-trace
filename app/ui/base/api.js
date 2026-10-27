// API: the only door to the core. Inside the app it calls the Rust commands through Tauri,
// in a plain browser it calls base/mock.js. Commands, arguments and answers: docs/ui-contract.md.
// Errors come back as short codes ("invalid_host", "resolve_failed", ...); parts turn them into text.
(function () {
  var T = window.__TAURI__;
  window.API = {
    inApp: !!T,
    call: function (cmd, args) {
      return T ? T.core.invoke(cmd, args || {}) : window.MOCK.call(cmd, args || {});
    },
    // Live events. "round": a target finished a round (payload: target id). Returns a promise of an unlisten function.
    on: function (name, cb) {
      return T ? T.event.listen(name, function (e) { cb(e.payload); }) : window.MOCK.on(name, cb);
    },
  };
})();
