// Whilo Trace, route map: the «new places database» dialog (MAPD). screens/trace.js opens it when geo_check offers a
// database (asked once per page load, when the map is first shown; docs/ui-contract.md). «Обновить» downloads it
// (geo_download) and shows the progress that the "geo" event reports; «Отменить» stops it (geo_cancel); a finished
// download says so for 2 s and closes, a failed one shows why. With no database at all the same dialog offers to
// download one. The shell is the report dialog's (.ms, .ms-scrim, .rp of parts/history/he.css), centred (map.css).
// While open it takes Esc (the button that is not the main one) and keeps Tab inside, then gives the focus back.
// MAPD.open(host, { offer: { month, bytes }, missing: no usable database now }).
(function () {
  'use strict';
  var EN = document.documentElement.dataset.lang === 'en';
  function L(ru, en) { return EN ? en : ru; }
  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; });
  }
  var MONTHS = [['январь', 'January'], ['февраль', 'February'], ['март', 'March'], ['апрель', 'April'], ['май', 'May'], ['июнь', 'June'],
    ['июль', 'July'], ['август', 'August'], ['сентябрь', 'September'], ['октябрь', 'October'], ['ноябрь', 'November'], ['декабрь', 'December']];
  function monthText(s) { var x = /^(\d{4})-(\d{2})$/.exec(s || ''), k = x ? +x[2] - 1 : -1; return MONTHS[k] ? L(MONTHS[k][0], MONTHS[k][1]) + ' ' + x[1] : String(s || ''); }
  function mb(bytes) { return Math.max(1, Math.round((bytes || 0) / 1e6)); }
  // Why a download did not work, by the core's error code (docs/ui-contract.md). none: there was no usable database
  // before (the dialog that offers to download one), so no old one stays: the map has no places yet.
  function errText(code, none) {
    var c = String(code == null ? '' : code.message || code), k = c.split(':')[0].trim();
    var left = none ? ['Карта пока без мест.', 'The map stays without places.'] : ['База осталась прежней.', 'The old database stays.'];
    var T = {
      geo_busy: ['База мест уже скачивается.', 'The places database is already downloading.'],
      geo_unavailable: ['В режиме имитации база мест не скачивается.', 'The places database is not downloaded in simulation mode.'],
      geo_invalid: none ? ['Скачанный файл не похож на базу мест. ' + left[0], 'The downloaded file is not a places database. ' + left[1]]
        : ['Скачанный файл не похож на базу мест, она осталась прежней.', 'The downloaded file is not a places database; the old one stays.'],
      geo_cancelled: ['Загрузка отменена.', 'The download was cancelled.'],
      net: ['Нет связи с db-ip.com. ' + left[0], 'Could not reach db-ip.com. ' + left[1]],
      http: ['Сервер db-ip.com ответил ошибкой' + (c.indexOf(':') > 0 ? ' ' + c.split(':')[1].trim() : '') + '. ' + left[0], 'The db-ip.com server answered with an error' + (c.indexOf(':') > 0 ? ' ' + c.split(':')[1].trim() : '') + '. ' + left[1]],
      io: ['Не удалось записать файл базы.', 'Could not write the database file.'],
      core_unavailable: ['Ядро программы не отвечает.', 'The app core does not answer.'],
    }[k];
    return T ? L(T[0], T[1]) : L('Не получилось: ', 'Failed: ') + c;
  }

  var cur = null; // the open dialog
  // One listener for the page: download progress goes to the open dialog.
  if (window.API) window.API.on('geo', function (d) { if (cur) cur.progress(d); });

  function open(host, o) {
    if (!host || !o || !o.offer || (host._rp && !host.hidden)) return null; // another dialog is open: not now
    var offer = o.offer, missing = !!o.missing, prev = document.activeElement, closed = false, timer = 0;
    var s = { step: 'ask', got: 0, total: null, err: null };
    // the City file plus about 5 MB for the AS base; no size when the server did not say (bytes null)
    var size = offer.bytes > 0 ? mb(offer.bytes + 5e6) : null;
    host.innerHTML = '<div class="ms mp-ms"><div class="ms-scrim"></div><div class="rp mp-dlg" role="dialog" aria-modal="true" aria-labelledby="mpDlgT"></div></div>';
    var root = host.firstChild, dlg = root.lastChild;
    var ctl = { kind: 'geo', close: close, progress: progress };
    host._rp = ctl;
    cur = ctl;

    function title() {
      if (s.step === 'done') return L('База мест обновлена', 'Places database updated');
      return missing ? L('Нужна база мест', 'Places database needed') : L('Вышла новая база мест', 'A new places database is out');
    }
    function body() {
      if (s.step === 'ask') {
        var ru = size ? ', около ' + size + ' МБ' : '', en = size ? ', about ' + size + ' MB' : '';
        return '<p>' + esc(missing
          ? L('Для карты нужна база мест DB-IP' + ru + '. Скачать?', 'The map needs the DB-IP places database' + en + '. Download it?')
          : L('База мест DB-IP за ' + monthText(offer.month) + ru + '. Обновить?', 'DB-IP places database for ' + monthText(offer.month) + en + '. Update?')) + '</p>';
      }
      if (s.step === 'run') {
        return '<p>' + esc(L('Скачивается база мест DB-IP за ' + monthText(offer.month) + '.', 'Downloading the DB-IP places database for ' + monthText(offer.month) + '.')) + '</p>' +
          '<div class="mp-prog" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-label="' + esc(L('Загрузка', 'Download')) + '"><i></i></div>' +
          '<p class="mp-prog-t"></p>';
      }
      if (s.step === 'done') return '<p>' + esc(L('Карта уже берет места из новой базы за ' + monthText(offer.month) + '.', 'The map now takes places from the new database for ' + monthText(offer.month) + '.')) + '</p>';
      return '<p>' + esc(s.err || errText('')) + '</p>';
    }
    function foot() {
      if (s.step === 'ask') {
        return '<button type="button" class="btn btn-primary" data-mpd="go">' + esc(missing ? L('Скачать', 'Download') : L('Обновить', 'Update')) + '</button>' +
          '<button type="button" class="btn" data-mpd="skip">' + esc(L('Пропустить', 'Skip')) + '</button>';
      }
      if (s.step === 'run') return '<button type="button" class="btn" data-mpd="cancel">' + esc(L('Отменить', 'Cancel')) + '</button>';
      return '<button type="button" class="btn" data-mpd="close">' + esc(L('Закрыть', 'Close')) + '</button>';
    }
    // The dialog is drawn once per step; the progress of a download only moves the bar and its text (meter()), so the
    // «Отменить» button under the pointer and the focus stay the same node.
    function paint() {
      if (closed) return;
      var focusAct = document.activeElement && dlg.contains(document.activeElement) ? document.activeElement.getAttribute('data-mpd') : null;
      dlg.innerHTML = '<div class="rp-head"><h2 id="mpDlgT">' + esc(title()) + '</h2></div><div class="rp-body">' + body() + '</div>' +
        '<div class="rp-foot"><span class="rp-count"></span>' + foot() + '</div>';
      if (s.step === 'run') meter();
      // The focus starts on «Пропустить»: the dialog comes unasked, a stray Enter must not start a big download.
      var b = (focusAct && dlg.querySelector('[data-mpd="' + focusAct + '"]')) || dlg.querySelector(s.step === 'ask' ? '[data-mpd="skip"]' : '.rp-foot .btn');
      if (b) b.focus({ preventScroll: true });
    }
    function meter() {
      var bar = dlg.querySelector('.mp-prog'), t = dlg.querySelector('.mp-prog-t');
      if (!bar || !t) return;
      var pct = s.total ? Math.min(100, Math.round(s.got / s.total * 100)) : null;
      bar.firstChild.style.width = (pct != null ? pct : 0) + '%';
      if (pct != null) bar.setAttribute('aria-valuenow', String(pct)); else bar.removeAttribute('aria-valuenow');
      t.textContent = s.total ? L(mb(s.got) + ' из ' + mb(s.total) + ' МБ', mb(s.got) + ' of ' + mb(s.total) + ' MB') : L(mb(s.got) + ' МБ', mb(s.got) + ' MB');
    }
    function close() {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      window.removeEventListener('keydown', onKey, true);
      host.hidden = true; host.innerHTML = '';
      if (host._rp === ctl) host._rp = null;
      if (cur === ctl) cur = null;
      if (prev && prev.isConnected && prev.focus) prev.focus({ preventScroll: true });
    }
    // Progress of the download ("geo" event: state, month, received, total, error).
    function progress(d) {
      if (closed || s.step !== 'run' || !d || (d.month && d.month !== offer.month)) return;
      if (d.state === 'running') { s.got = d.received || 0; s.total = d.total || null; meter(); }
      else if (d.state === 'done') {
        s.step = 'done'; paint(); timer = setTimeout(close, 2000);
        if (window.Live && window.Live.refresh) window.Live.refresh().catch(function () { /* Live logs it */ }); // the new month at once
      }
      else if (d.state === 'failed') { s.step = 'fail'; s.err = errText(d.error, missing); paint(); }
      else if (d.state === 'cancelled') close();
    }
    function go() {
      s.step = 'run'; s.got = 0; s.total = null;
      paint();
      window.API.call('geo_download', { month: offer.month }).catch(function (e) {
        if (closed) return;
        console.error('geo_download failed', e);
        s.step = 'fail'; s.err = errText(e, missing);
        paint();
      });
    }
    function cancel() {
      window.API.call('geo_cancel', {}).catch(function (e) { console.error('geo_cancel failed', e); });
      close();
    }
    // The button that is not the main one: skip the offer, cancel the download, or close.
    function secondary() { if (s.step === 'run') cancel(); else close(); }
    function onKey(e) {
      if (closed || e.isComposing) return;
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); secondary(); return; }
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
      // A click beside the dialog skips the offer or closes a result; it never cancels a download on its way.
      if (t === root || t.classList.contains('ms-scrim')) { if (s.step !== 'run') close(); return; }
      var b = t.closest && t.closest('[data-mpd]');
      if (!b) return;
      var a = b.getAttribute('data-mpd');
      if (a === 'go') go();
      else if (a === 'cancel') cancel();
      else close();
    });
    window.addEventListener('keydown', onKey, true);
    host.hidden = false;
    paint();
    return ctl;
  }

  window.MAPD = { open: open };
})();
