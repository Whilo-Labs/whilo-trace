// Whilo Trace, route map: the «Сохранить картинку» dialog (MAP.picture(host), opened from the map's head). The
// picture is for publication: the map with its callouts and the hop ribbon, always dark, your network and your
// provider's first hops hidden (MAP.png in parts/map/map.js), 1200×675 (the ribbon right of the map) or 1080×1080 (the
// ribbon under it). The preview is the very picture that is saved. «Сохранить» writes it with save_image (flat
// arguments {dir, name, pngBase64}, docs/ui-contract.md) into the reports' folder, Documents\Whilo Trace, or the one
// picked with «Изменить…» (pick_folder); then «Открыть папку» (open_folder). The shell is the report dialog's (.ms,
// .ms-scrim, .rp of parts/history/he.css), centred (map.css). While open it takes Esc, keeps Tab inside and then
// gives the focus back.
(function () {
  'use strict';
  if (!window.MAP) return;
  var EN = document.documentElement.dataset.lang === 'en';
  function L(ru, en) { return EN ? en : ru; }
  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; });
  }
  var SEP = /Windows/i.test(navigator.userAgent) ? '\\' : '/';
  function svg(d) { return '<svg class="ico" viewBox="0 0 16 16" aria-hidden="true" focusable="false">' + d + '</svg>'; }
  var I = {
    folder: svg('<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.7l1.5 1.6h4.8A1.5 1.5 0 0 1 14 6.1v5.4a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z"/>'),
    check: svg('<path d="M3.5 8.5l3 3 6-7"/>'),
  };
  var SIZES = [['wide', '1200×675'], ['square', '1080×1080']];
  function two(x) { return (x < 10 ? '0' : '') + x; }
  // whilo-trace-map-8.8.8.8-20261027-160000: the target and the end of the data's window, local time (the core
  // cleans the name again and adds -2, -3 when the file is there). A destination of your network typed as an address
  // is hidden on the picture, so it stays out of the name too (route).
  function fileName() {
    var d = new Date(WT.toMs || WT.nowMs || Date.now()), tr = WT.trace;
    var row = tr ? (WT.targets || []).filter(function (x) { return x.id === tr.target; })[0] : null;
    var dh = tr ? (tr.hops || []).filter(function (h) { return h.dest; })[0] : null;
    var gone = !!(row && dh && dh.geo && dh.geo.level === 'own' && /^[0-9.]+$|:/.test(String(row.host)));
    var host = row && !gone ? String(row.host).replace(/[^A-Za-z0-9.-]/g, '_') : 'route';
    return 'whilo-trace-map-' + host + '-' + d.getFullYear() + two(d.getMonth() + 1) + two(d.getDate()) + '-' + two(d.getHours()) + two(d.getMinutes()) + two(d.getSeconds());
  }
  function errText(e) {
    var info = window.HE && HE.errorInfo ? HE.errorInfo(e) : null;
    return L('Не удалось сохранить картинку. ', 'Could not save the picture. ') + (info ? info.text : String(e && e.message || e));
  }
  function dirOf(p) { var s = String(p || ''), i = Math.max(s.lastIndexOf('\\'), s.lastIndexOf('/')); return i > 0 ? s.slice(0, i) : s; }
  // A path in the report dialog's field: its beginning shrinks with an ellipsis, the last name stays visible.
  function pathHtml(full) {
    var s = String(full), i = Math.max(s.lastIndexOf('\\'), s.lastIndexOf('/'));
    return '<span class="rp-path-f" title="' + esc(s) + '">' + I.folder + '<span class="mono rp-path-pre">' + esc(s.slice(0, i + 1)) + '</span>' +
      '<span class="mono rp-path-name">' + esc(s.slice(i + 1)) + '</span></span>';
  }

  function open(host) {
    if (!host || (host._rp && !host.hidden)) return null; // another dialog is open
    var prev = document.activeElement, closed = false;
    var s = { size: 'wide', dir: null, url: null, busy: false, err: null, res: null };
    host.innerHTML = '<div class="ms mp-ms"><div class="ms-scrim"></div><div class="rp mp-dlg mp-pic" role="dialog" aria-modal="true" aria-labelledby="mpPicT"></div></div>';
    var root = host.firstChild, dlg = root.lastChild;
    var ctl = { kind: 'picture', close: close };
    host._rp = ctl;

    // The picture of the map as it is now, for the chosen size; null when the route has no places.
    function render() {
      try { s.url = MAP.png(null, { size: s.size, theme: 'dark', mask: true }); } catch (e) { console.error(e); s.url = null; }
    }
    function formHtml() {
      var seg = '<div class="seg" role="group" aria-label="' + esc(L('Размер картинки', 'Picture size')) + '">' + SIZES.map(function (z) {
        return '<button type="button" data-mpp="size" data-size="' + z[0] + '" aria-pressed="' + (s.size === z[0]) + '">' + esc(z[1]) + '</button>';
      }).join('') + '</div>';
      var view = s.url ? '<img src="' + esc(s.url) + '" alt="' + esc(L('Картинка маршрута', 'Route picture')) + '">'
        : '<p>' + esc(L('Мест для этого маршрута нет, картинку не из чего сделать.', 'This route has no places, there is nothing to make a picture of.')) + '</p>';
      var dir = s.dir ? s.dir : L('Документы', 'Documents') + SEP + 'Whilo Trace';
      return '<div class="rp-head"><h2 id="mpPicT">' + esc(L('Картинка маршрута', 'Route picture')) + '</h2></div>' +
        '<div class="rp-body">' +
        '<div class="mp-pic-bar">' + seg + '<span class="mp-pic-px">' + esc(L('пикселей, PNG', 'pixels, PNG')) + '</span></div>' +
        '<div class="mp-pic-view">' + view + '</div>' +
        '<p class="mp-pic-note">' + esc(L('Адреса вашей сети и первых хопов провайдера скрыты, дом показан точкой страны, а не городом. Картинка всегда темная.',
          "Addresses of your network and your provider's first hops are hidden, the home is shown at the country's point, not its city. The picture is always dark.")) + '</p>' +
        '<div class="rp-path">' + pathHtml(dir) + '<button type="button" class="btn" data-mpp="dir">' + esc(L('Изменить…', 'Change…')) + '</button></div>' +
        '</div>' +
        '<div class="rp-foot"><span class="rp-count' + (s.err ? ' warn' : '') + '">' + esc(s.err || (s.busy ? L('Сохраняю картинку…', 'Saving the picture…') : '')) + '</span>' +
        '<button type="button" class="btn btn-primary" data-mpp="save"' + (s.url && !s.busy ? '' : ' disabled') + '>' + esc(L('Сохранить', 'Save')) + '</button>' +
        '<button type="button" class="btn" data-mpp="close">' + esc(L('Отмена', 'Cancel')) + '</button></div>';
    }
    function savedHtml() {
      var p = String(s.res.path || '');
      return '<div class="rp-head"><div class="rp-ok">' + I.check + '<h2 id="mpPicT">' + esc(L('Картинка сохранена', 'Picture saved')) + '</h2></div></div>' +
        '<div class="rp-body"><div class="rp-path mp-pic-saved">' + pathHtml(p) + '</div></div>' +
        '<div class="rp-foot"><span class="rp-count' + (s.err ? ' warn' : '') + '">' + esc(s.err || '') + '</span>' +
        '<button type="button" class="btn btn-primary" data-mpp="open">' + I.folder + esc(L('Открыть папку', 'Open folder')) + '</button>' +
        '<button type="button" class="btn" data-mpp="close">' + esc(L('Готово', 'Done')) + '</button></div>';
    }
    // The dialog again; the focus stays on the same action when it is still there.
    function paint(focusAct) {
      if (closed) return;
      var ae = document.activeElement, keep = focusAct || (ae && dlg.contains(ae) ? ae.getAttribute('data-mpp') + (ae.getAttribute('data-size') || '') : null);
      dlg.innerHTML = s.res ? savedHtml() : formHtml();
      var b = null;
      if (keep) [].forEach.call(dlg.querySelectorAll('[data-mpp]'), function (x) { if (!b && x.getAttribute('data-mpp') + (x.getAttribute('data-size') || '') === keep && !x.disabled) b = x; });
      if (!b) b = dlg.querySelector(s.res ? '[data-mpp="open"]' : '[data-mpp="save"]:not([disabled])') || dlg.querySelector('[data-mpp="close"]');
      if (b) b.focus({ preventScroll: true });
    }
    function close() {
      if (closed) return;
      closed = true;
      window.removeEventListener('keydown', onKey, true);
      host.hidden = true; host.innerHTML = '';
      if (host._rp === ctl) host._rp = null;
      if (prev && prev.isConnected && prev.focus) prev.focus({ preventScroll: true });
    }
    function save() {
      if (s.busy || !s.url) return;
      s.busy = true; s.err = null;
      paint();
      window.API.call('save_image', { dir: s.dir, name: fileName(), pngBase64: s.url }).then(function (res) {
        if (closed) return;
        s.busy = false; s.res = res || { path: '' };
        paint('open');
      }, function (e) {
        if (closed) return;
        console.error('save_image failed', e);
        s.busy = false; s.err = errText(e);
        paint('save');
      });
    }
    function pickDir() {
      window.API.call('pick_folder', {}).then(function (p) {
        if (closed) return;
        if (p) { s.dir = String(p); s.err = null; }
        paint('dir');
      }, function (e) { if (closed) return; s.err = errText(e); paint('dir'); });
    }
    function openDir() {
      window.API.call('open_folder', { path: dirOf(s.res.path) }).then(function () {
        if (!closed && s.err) { s.err = null; paint('open'); }
      }, function (e) {
        if (closed) return;
        var info = window.HE && HE.errorInfo ? HE.errorInfo(e) : null;
        s.err = L('Не удалось открыть папку. ', 'Could not open the folder. ') + (info ? info.text : String(e));
        paint('open');
      });
    }
    function onKey(e) {
      if (closed || e.isComposing) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); return; }
      if (e.key !== 'Tab') return;
      var list = [].slice.call(dlg.querySelectorAll('button:not([disabled])')), ae = document.activeElement;
      if (!list.length) return;
      var first = list[0], last = list[list.length - 1];
      if (!dlg.contains(ae)) { e.preventDefault(); (e.shiftKey ? last : first).focus(); }
      else if (e.shiftKey && ae === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && ae === last) { e.preventDefault(); first.focus(); }
    }
    root.addEventListener('click', function (e) {
      e.stopPropagation();
      var t = e.target;
      // a click beside the dialog closes it, except while the picture is being written
      if (t === root || t.classList.contains('ms-scrim')) { if (!s.busy) close(); return; }
      var b = t.closest && t.closest('[data-mpp]');
      if (!b || b.disabled) return;
      var a = b.getAttribute('data-mpp');
      if (a === 'size') {
        var z = b.getAttribute('data-size');
        if (z !== s.size && !s.busy) { s.size = z; s.err = null; render(); paint(); }
      } else if (a === 'save') save();
      else if (a === 'dir') pickDir();
      else if (a === 'open') openDir();
      else close();
    });
    root.addEventListener('keydown', function (e) { e.stopPropagation(); });
    window.addEventListener('keydown', onKey, true);
    render();
    host.hidden = false;
    paint();
    return ctl;
  }

  window.MAP.picture = open;
})();
