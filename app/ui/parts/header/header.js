// Whilo Trace, header part (WTH): brand, the add-target form (field with recent inputs, probe types,
// interval of the new target, "Add"), time window, legend and the settings card on the right (variant 6B).
// Data: window.WT (base/wt.js) and Live.state (base/live.js). Actions go to the core through window.API
// (base/api.js, commands and error codes in docs/ui-contract.md).
// The screen hears about changes through callbacks passed to WTH.header(o) or WTH.bindAll(o):
//   onAdded({ id, ids, host, probes, intervalS }), onWindow(key), onSettings(settings, patch), onInterval(intervalS).
// WTH.add(host) adds a target the way «Добавить» does (the first-run quick start uses it), WTH.interval() is the
// interval picked for a new target. Error texts come from HE.errorInfo (parts/history) when it is on the page.
// New data after add_target and set_settings comes by itself (base/live.js refreshes after such commands);
// without onWindow the part sets the window through Live.set({ window }). Theme and language go through
// WTTheme (base/theme.js): the theme applies at once, a new language reloads the page.
// Texts are written in the current language and carry data-en.
(function () {
  'use strict';
  var WT = window.WT;
  var BASE = new URL('../../base/', document.currentScript.src).href;
  var DEFAULT_IV = 2.5; // seconds, interval of a new target

  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  function isEn() { return document.documentElement.lang === 'en'; }
  function L(ru, en) { return isEn() ? en : ru; }
  function T(ru, en, cls) { return '<span' + (cls ? ' class="' + cls + '"' : '') + ' data-en="' + esc(en) + '">' + esc(L(ru, en)) + '</span>'; }
  function A(name, ru, en) { return ' ' + name + '="' + esc(L(ru, en)) + '" data-en-' + name + '="' + esc(en) + '"'; }
  function ruNum(n) { return String(n).replace('.', ','); }
  function secs(n) { return { ru: ruNum(n) + ' с', en: String(n) + ' s' }; }
  function $(s, r) { return (r || document).querySelector(s); }
  function $$(s, r) { return [].slice.call((r || document).querySelectorAll(s)); }

  // Line icons, stroke currentColor.
  var SVG = {
    plus: '<path d="M8 3.5v9M3.5 8h9"/>',
    chev: '<path d="M4.5 6.5 8 10l3.5-3.5"/>',
    check: '<path d="M3.5 8.5l3 3 6-7"/>',
    close: '<path d="M4.5 4.5l7 7M11.5 4.5l-7 7"/>',
    clock: '<circle cx="8" cy="8" r="5.75"/><path d="M8 5v3.25l2.25 1.5"/>',
    alert: '<circle cx="8" cy="8" r="6"/><path d="M8 4.75v3.75M8 11.1v.1"/>',
    target: '<circle cx="8" cy="8" r="5.75"/><circle cx="8" cy="8" r="2.25"/>',
    gear: '<path d="M6.66 3.18L6.84 1.4L9.16 1.4L9.34 3.18A5 5 0 0 1 10.46 3.65L11.84 2.51L13.49 4.16L12.35 5.54A5 5 0 0 1 12.82 6.66L14.6 6.84L14.6 9.16L12.82 9.34A5 5 0 0 1 12.35 10.46L13.49 11.84L11.84 13.49L10.46 12.35A5 5 0 0 1 9.34 12.82L9.16 14.6L6.84 14.6L6.66 12.82A5 5 0 0 1 5.54 12.35L4.16 13.49L2.51 11.84L3.65 10.46A5 5 0 0 1 3.18 9.34L1.4 9.16L1.4 6.84L3.18 6.66A5 5 0 0 1 3.65 5.54L2.51 4.16L4.16 2.51L5.54 3.65A5 5 0 0 1 6.66 3.18Z"/><circle cx="8" cy="8" r="2.1"/>',
  };
  function I(name, cls) { return '<svg class="i' + (cls ? ' ' + cls : '') + '" viewBox="0 0 16 16" aria-hidden="true">' + SVG[name] + '</svg>'; }

  // ---------- State of the form (kept across re-renders) ----------
  var ST = {
    probes: null,          // probe buttons switched on (ids of PROBES)
    interval: DEFAULT_IV,  // interval of the new target, s
    adding: null,          // promise of the add on its way
    pointerDown: false,    // a button is pressed outside the header: a drag or a period being picked
    recent: [],            // last answer of recent_inputs
    wantRecent: false,     // the user asked for the recent list (focus, typing, arrow)
    active: -1,            // highlighted recent item (index among the visible ones)
    ivActive: -1,          // highlighted interval item
    silentFocus: false,    // programmatic focus: no recent list
    lastInput: 0,          // time of the last pointer or key press: focus without one is programmatic
    recentSeq: 0,
    thLocal: null,         // thresholds typed but not saved yet
    thTimer: null,
    focusOnBind: false,
  };
  var CB = {};
  function keep(o) {
    ['onAdded', 'onWindow', 'onSettings', 'onInterval'].forEach(function (k) { if (o && typeof o[k] === 'function') CB[k] = o[k]; });
  }
  function noop() {}

  function call(cmd, args) {
    try { return Promise.resolve(window.API.call(cmd, args || {})); } catch (e) { return Promise.reject(e); }
  }
  function settings() { return WT.settings || {}; }
  function thresholds() {
    if (ST.thLocal) return ST.thLocal;
    var s = settings();
    return { warn: +s.warnMs || 100, bad: +s.badMs || 200 };
  }
  function curWindow() {
    var w = window.Live && window.Live.state && window.Live.state.window;
    return w || settings().window || '10m';
  }
  function curTheme() { var t = document.documentElement.dataset.theme; return t === 'light' || t === 'dark' ? t : 'system'; }
  function curLang() { return isEn() ? 'en' : 'ru'; }

  // Swatches of the timeline marks: a small chart well (--well) with the same mark as on the chart.
  function sw(kind) {
    var body = '';
    if (kind === 'warn') body = '<rect class="f-warn" width="24" height="14"/>';
    else if (kind === 'bad') body = '<rect class="f-bad" width="24" height="14"/>';
    else if (kind === 'line') body = '<path class="s-line" d="M0 10L5 9L9 11L13 6L17 8L24 7"/>';
    else if (kind === 'loss') body = '<rect class="f-loss" x="9" width="6" height="14"/>';
    else if (kind === 'partial') body = '<rect class="f-partial" x="3" y="10" width="3" height="4"/><rect class="f-partial" x="10.5" y="10" width="3" height="4"/><rect class="f-partial" x="18" y="10" width="3" height="4"/>';
    else if (kind === 'silent') {
      var d = '';
      for (var x = -14; x < 24; x += 5) d += 'M' + x + ' 14L' + (x + 14) + ' 0';
      body = '<path class="s-silent" d="' + d + '"/>';
    }
    return '<span class="wth-sw" aria-hidden="true"><svg viewBox="0 0 24 14" preserveAspectRatio="none">' + body + '</svg></span>';
  }

  // ---------- Legend ----------
  // Thresholds come from WT.settings (or the values being typed in the settings card).
  function legend() {
    var st = WT.statuses, th = thresholds(), a = th.warn, b = th.bad;
    var silentRu = st.silent.ru.replace('{probe}', 'пробы этого типа');
    var silentEn = st.silent.en.replace('{probe}', 'this probe type');
    function li(inner, hint, cls) {
      return '<li class="has-tip' + (cls ? ' ' + cls : '') + '" tabindex="0">' + inner +
        '<span class="wth-tip to-right" role="tooltip">' + T(hint[0], hint[1]) + '</span></li>';
    }
    var h = '<ul class="wth-legend" data-warn="' + a + '" data-bad="' + b + '"' + A('aria-label', 'Обозначения на таймлайнах', 'Timeline legend') + '>';
    h += li(T('задержка', 'Latency', 'lg-word') + sw('line') + T(String(a), String(a), 'num') + sw('warn') + T(String(b), String(b), 'num') + sw('bad') + T('мс', 'ms'),
      ['Линия задержки на фоне по порогам: до ' + a + ' мс без заливки, ' + a + '-' + b + ' мс и выше ' + b + ' мс своим цветом. Пороги меняются в настройках.',
        'Latency line over a background by thresholds: no tint below ' + a + ' ms, its own tint for ' + a + '-' + b + ' ms and above ' + b + ' ms. Change thresholds in Settings.'], 'lg-scale');
    h += li(sw('loss') + T('потеря', 'Loss'),
      ['Ответ на пробу не пришел. Столбец на всю высоту графика.', 'The probe got no reply. The bar fills the whole chart height.']);
    h += li(sw('partial') + T(st.partial.ru, st.partial.en), [st.partial.hint_ru, st.partial.hint_en]);
    h += li(sw('silent') + T(silentRu, silentEn), [st.silent.hint_ru, st.silent.hint_en]);
    return h + '</ul>';
  }

  // ---------- Add form ----------
  function field() {
    var h = '<div class="wth-field"><div class="wth-field-box">' + I('target') +
      '<input class="wth-input" type="text" spellcheck="false" autocomplete="off" autocapitalize="off" role="combobox" aria-autocomplete="list"' +
      ' aria-expanded="false" aria-controls="wth-recent" aria-describedby="wth-error"' +
      A('placeholder', 'Имя или IP, например 1.1.1.1', 'Name or IP, e.g. 1.1.1.1') + A('aria-label', 'Имя или IP цели', 'Target name or IP') + '></div>';
    h += '<div class="wth-error" id="wth-error" role="alert" hidden></div>';
    h += '<div class="wth-menu wth-history" hidden><div class="wth-menu-head" aria-hidden="true">' + T('Недавние', 'Recent') + '</div>' +
      '<div class="wth-recent" id="wth-recent" role="listbox"' + A('aria-label', 'Недавние', 'Recent') + '></div></div>';
    return h + '</div>';
  }

  // Second name of a recent host from the rows on screen: the DNS name of an IP or the address of a name.
  function asideOf(host) {
    var t = (WT.targets || []).filter(function (x) { return !x.hopOf && x.host === host; })[0];
    if (!t) return '';
    if (t.name && t.name !== host) return t.name;
    return t.ip && t.ip !== host ? t.ip : '';
  }
  function recentItems(list) {
    return list.map(function (host, i) {
      var aside = asideOf(host);
      return '<div class="wth-menu-item" role="option" id="wth-recent-' + i + '" aria-selected="false" data-host="' + esc(host) + '">' + I('clock') +
        T(host, host, 'mono') + (aside ? T(aside, aside, 'aside mono') : '') + '</div>';
    }).join('');
  }

  // Probe types. The engine sends only those listed in WT.probesAvailable; the rest stay visible but off.
  var PROBES = [['icmp', 'ICMP'], ['tcp', 'TCP'], ['udp', 'UDP']];
  function offTip(p) { return ['Пробы ' + p[1] + ' появятся позже', p[1] + ' probes are coming later']; }
  // Probe id the core knows for a button ("tcp" may come as "tcp443"), or null while the engine does not send it.
  function probeId(p) {
    var ids = WT.probesAvailable && WT.probesAvailable.length ? WT.probesAvailable : ['icmp'];
    for (var i = 0; i < ids.length; i++) if (ids[i] === p || ids[i].indexOf(p) === 0) return ids[i];
    return null;
  }
  // Buttons switched on; at least one available type stays on.
  function selected() {
    var on = (ST.probes || []).filter(function (p) { return probeId(p); });
    if (!on.length) { var first = PROBES.filter(function (p) { return probeId(p[0]); })[0]; if (first) on = [first[0]]; }
    ST.probes = on;
    return on;
  }
  function probeBtn(p) {
    var off = !probeId(p[0]), on = !off && ST.probes.indexOf(p[0]) >= 0, tip = '';
    if (off) { var t = offTip(p); tip = '<span class="wth-tip" role="tooltip">' + T(t[0], t[1]) + '</span>'; }
    return '<button type="button" class="wth-opt' + (off ? ' has-tip' : '') + '" data-probe="' + p[0] + '" aria-pressed="' + on + '"' +
      (off ? ' aria-disabled="true"' : '') + '>' + (off ? '' : I(on ? 'check' : 'plus')) + T(p[1], p[1]) + tip + '</button>';
  }
  function probeGroup() {
    selected();
    return '<div class="wth-group" role="group"' + A('aria-label', 'Типы проб', 'Probe types') + '>' + T('Пробы', 'Probes', 'wth-label wth-wide') +
      '<div class="seg wth-probes">' + PROBES.map(probeBtn).join('') + '</div>' +
      '<label class="wth-port"' + (ST.probes.indexOf('tcp') >= 0 ? '' : ' hidden') + '>' + T('порт', 'port') +
      '<input class="num" inputmode="numeric" maxlength="5" value="443"' + A('aria-label', 'Порт TCP', 'TCP port') + '></label></div>';
  }

  function ivLabel(n) { var s = secs(n); return { ru: 'Интервал опроса новой цели: ' + s.ru, en: 'Probe interval of the new target: ' + s.en }; }
  function intervalSelect() {
    var cur = ST.interval, s = secs(cur), lb = ivLabel(cur);
    var h = '<div class="wth-select">';
    h += '<button type="button" class="wth-select-btn" aria-haspopup="listbox" aria-expanded="false" aria-controls="wth-iv-menu"' + A('aria-label', lb.ru, lb.en) + '>' +
      I('clock', 'i-clock') + T('Интервал', 'Interval', 'wth-label wth-wide') + T(s.ru, s.en, 'num wth-iv-val') + I('chev', 'i-chev') + '</button>';
    h += '<div class="wth-menu wth-intervals" id="wth-iv-menu" role="listbox"' + A('aria-label', 'Интервал опроса', 'Probe interval') + ' hidden>';
    WT.intervals.forEach(function (n) {
      var on = n === cur, v = secs(n);
      h += '<div class="wth-menu-item' + (on ? ' is-on' : '') + '" role="option" aria-selected="' + on + '" data-iv="' + n + '">' +
        '<span class="ck">' + (on ? I('check') : '') + '</span>' + T(v.ru, v.en, 'num') + (n === DEFAULT_IV ? T('по умолчанию', 'default', 'aside') : '') + '</div>';
    });
    return h + '</div></div>';
  }

  function addBtn() { return '<button type="button" class="btn btn-primary wth-add-btn">' + I('plus') + T('Добавить', 'Add') + '</button>'; }

  function langSeg() {
    return '<div class="seg wth-lang" role="group"' + A('aria-label', 'Язык интерфейса', 'Interface language') + '>' + [['ru', 'RU'], ['en', 'EN']].map(function (l) {
      return '<button type="button" class="wth-opt" data-lang-set="' + l[0] + '" aria-pressed="' + (curLang() === l[0]) + '">' + T(l[1], l[1]) + '</button>';
    }).join('') + '</div>';
  }

  function gearBtn() {
    return '<button type="button" class="wth-icon-btn wth-gear has-tip" aria-expanded="false" aria-controls="wth-settings"' + A('aria-label', 'Настройки', 'Settings') + '>' +
      I('gear') + '<span class="wth-tip to-right" role="tooltip">' + T('Настройки', 'Settings') + '</span></button>';
  }

  function windowSeg() {
    var cur = curWindow();
    var h = '<div class="wth-group">';
    h += '<span class="wth-label has-tip" tabindex="0">' + T('Окно', 'Window') +
      '<span class="wth-tip" role="tooltip">' + T('Окно времени сразу для всех таймлайнов', 'Time window for all timelines at once') + '</span></span>';
    h += '<div class="seg wth-window" role="group"' + A('aria-label', 'Окно времени', 'Time window') + '>' + WT.windows.map(function (w) {
      return '<button type="button" class="wth-opt" data-win="' + w[0] + '" aria-pressed="' + (w[0] === cur) + '">' + T(w[1], w[2]) + '</button>';
    }).join('') + '</div>';
    return h + '</div>';
  }

  // ---------- Settings card (right of the data, variant 6B) ----------
  // The screen places it next to the data: WTH.settingsPanel() into .app-body. Closed unless hidden === false.
  function settingsPanel(kind, hidden) {
    var th = thresholds(), theme = curTheme(), lang = curLang();
    var h = '<div class="wth-side" id="wth-settings" role="dialog"' + A('aria-label', 'Настройки', 'Settings') + (hidden === false ? '' : ' hidden') + '>';
    h += '<div class="wth-pop-head"><h2>' + T('Настройки', 'Settings') + '</h2><button type="button" class="wth-ghost wth-close"' + A('aria-label', 'Закрыть', 'Close') + '>' + I('close') + '</button></div>';
    h += '<div class="wth-set"><div class="wth-set-label">' + T('Пороги задержки', 'Latency thresholds') + '</div><div class="wth-th">' +
      '<label class="wth-num">' + sw('warn') + T('от', 'from') + '<input data-th="warn" inputmode="numeric" maxlength="4" value="' + th.warn + '"' + A('aria-label', 'Первый порог, мс', 'First threshold, ms') + '>' + T('мс', 'ms') + '</label>' +
      '<label class="wth-num">' + sw('bad') + T('от', 'from') + '<input data-th="bad" inputmode="numeric" maxlength="4" value="' + th.bad + '"' + A('aria-label', 'Второй порог, мс', 'Second threshold, ms') + '>' + T('мс', 'ms') + '</label></div>' +
      '<p class="wth-hint">' + T('Фон таймлайнов окрашивается, когда задержка выше порога.', 'Timeline background is tinted when latency is above a threshold.') + '</p></div>';
    h += '<div class="wth-set"><div class="wth-set-label">' + T('Тема', 'Theme') + '</div><div class="seg" role="group"' + A('aria-label', 'Тема', 'Theme') + '>' +
      [['system', 'Как в системе', 'System'], ['light', 'Светлая', 'Light'], ['dark', 'Темная', 'Dark']].map(function (t) {
        return '<button type="button" data-theme-set="' + t[0] + '" aria-pressed="' + (theme === t[0]) + '">' + T(t[1], t[2]) + '</button>';
      }).join('') + '</div></div>';
    h += '<div class="wth-set"><div class="wth-set-label">' + T('Язык', 'Language') + '</div><div class="seg" role="group"' + A('aria-label', 'Язык', 'Language') + '>' +
      [['ru', 'Русский'], ['en', 'English']].map(function (l) {
        return '<button type="button" data-lang-set="' + l[0] + '" aria-pressed="' + (lang === l[0]) + '">' + T(l[1], l[1]) + '</button>';
      }).join('') + '</div></div>';
    return h + '</div>';
  }

  // ---------- Whole header ----------
  // o: onAdded, onWindow, onSettings (see the top of the file), interval (of the new target, s),
  //    focus (put the caret in the field after bindAll, without the recent list).
  function header(o) {
    o = o || {};
    keep(o);
    if (o.focus) ST.focusOnBind = true;
    if (WT.intervals.indexOf(o.interval) >= 0) ST.interval = o.interval;
    var h = '<header class="wth" data-live="1" data-settings="side">';
    h += '<div class="wth-row wth-row-top">';
    h += '<div class="wth-brand"><img src="' + BASE + 'icon.png" alt="" width="28" height="28">' + T('Whilo Trace', 'Whilo Trace', 'wth-brand-name') + '</div>';
    h += '<form class="wth-add" autocomplete="off" novalidate' + A('aria-label', 'Добавить цель', 'Add target') + '>';
    h += field() + probeGroup() + intervalSelect() + addBtn();
    h += '</form><div class="wth-spacer"></div>' + langSeg() + gearBtn() + '</div>';
    h += '<div class="wth-row wth-row-view">' + windowSeg() + '<div class="wth-spacer"></div>' + legend() + '</div>';
    return h + '</header>';
  }

  // ---------- Errors of add_target ----------
  // Codes from docs/ui-contract.md; "db: <detail>" and "io: <detail>" carry a detail. "empty" is the header's own.
  function codeOf(err) { return String(err && typeof err === 'object' ? (err.code || err.message || '') : err == null ? '' : err); }
  // The header's own texts, while the history part is not on the page. arg is shown in mono after the text.
  function ownErr(c, host) {
    var m = /^(db|io):\s*([\s\S]*)$/.exec(c), detail = '';
    if (m) { c = m[1]; detail = m[2]; }
    switch (c) {
      case 'empty': return { ru: 'Введите имя или IP-адрес', en: 'Enter a name or an IP address' };
      case 'invalid_host': return { ru: 'Неверное имя или IP-адрес', en: 'Invalid name or IP address', arg: host };
      case 'resolve_failed': return { ru: 'Не удалось найти хост', en: 'Could not find host', arg: host };
      case 'no_ipv4': return { ru: 'Нет адреса IPv4', en: 'No IPv4 address', arg: host };
      case 'probe_unavailable': return { ru: 'Этот тип пробы пока недоступен', en: 'This probe type is not available yet' };
      case 'invalid_interval': return { ru: 'Такой интервал не поддерживается', en: 'This interval is not supported' };
      case 'not_implemented': return { ru: 'Добавление целей пока не работает', en: 'Adding targets does not work yet' };
      case 'db': return { ru: 'Не удалось сохранить цель', en: 'Could not save the target', arg: detail };
      case 'io': return { ru: 'Ошибка ввода-вывода', en: 'Input/output error', arg: detail };
      default: return { ru: 'Не удалось добавить цель', en: 'Could not add the target', arg: c };
    }
  }
  // A text of HE.errorInfo: a string in the interface language or [ru, en].
  function inLang(v) {
    if (Array.isArray(v)) return v[0] || v[1] ? { text: L(v[0] || v[1], v[1] || v[0]), en: v[1] || null } : null;
    return typeof v === 'string' && v ? { text: v, en: null } : null;
  }
  function cap(s) { return s ? s.charAt(0).toUpperCase() + s.slice(1) : s; }
  var HOST_CODES = { invalid_host: 1, resolve_failed: 1, no_ipv4: 1 };
  // One text per code for every part (HE.errorInfo). An error about the host or the probe type reads as its short
  // status, the same as on a row with this error, followed by the host («Не удалось найти хост example.con»);
  // other codes read as the full text, which carries the detail. The full text is also the hint of the line.
  // { text, en (null when only the interface language is known), full, arg (the host in mono, unless the text has it) }
  function errMsg(err, host, probe) {
    var raw = codeOf(err), HE = window.HE, info = null;
    if (raw !== 'empty' && HE && typeof HE.errorInfo === 'function') {
      try { info = HE.errorInfo(raw, { host: host, probe: probe }); } catch (x) { console.error(x); }
    }
    if (info) {
      var c = info.code || raw.split(':')[0].trim(), title = inLang(info.title), hint = inLang(info.hint);
      var full = inLang(info.text) || (title && { text: title.text + (hint ? ' ' + hint.text : ''), en: null });
      var status = inLang(info.status), short = (HOST_CODES[c] || c === 'probe_unavailable') ? status || full : full || status;
      if (short) {
        return { text: cap(short.text), en: short.en && cap(short.en), full: full ? full.text : '',
          arg: HOST_CODES[c] && host && short.text.indexOf(host) < 0 ? host : '' };
      }
    }
    var own = ownErr(raw, host);
    return { text: L(own.ru, own.en), en: own.en, full: '', arg: own.arg || '' };
  }

  // ---------- Behavior ----------
  function fieldParts() {
    var f = $('.wth-field');
    return f && { fld: f, input: $('.wth-input', f), err: $('.wth-error', f), menu: $('.wth-history', f), list: $('.wth-recent', f) };
  }
  function setFieldIcon(name) { var x = fieldParts(), old = x && $('.wth-field-box > svg', x.fld); if (old) old.outerHTML = I(name); }

  function showError(code, host, probe) {
    var x = fieldParts();
    if (!x) return;
    var m = errMsg(code, host, probe), hint = m.full && m.full !== m.text ? m.full : m.text;
    x.err.innerHTML = '<span' + (m.en ? ' data-en="' + esc(m.en) + '"' : '') + ' title="' + esc(hint) + '">' + esc(m.text) + '</span>' +
      (m.arg ? '<span class="mono wth-error-host" title="' + esc(m.arg) + '">' + esc(m.arg) + '</span>' : '');
    x.err.hidden = false;
    if (!x.fld.classList.contains('is-error')) { x.fld.classList.add('is-error'); setFieldIcon('alert'); }
    x.input.setAttribute('aria-invalid', 'true');
    showRecent(false);
  }
  function clearError() {
    var x = fieldParts();
    if (!x || !x.fld.classList.contains('is-error')) return;
    x.fld.classList.remove('is-error'); x.err.hidden = true; x.err.innerHTML = '';
    x.input.removeAttribute('aria-invalid');
    setFieldIcon('target');
  }

  // Recent inputs: asked from the core on every focus of the field, filtered by what is typed.
  function openRecent() { ST.wantRecent = true; showRecent(); loadRecent(); }
  function loadRecent() {
    var seq = ++ST.recentSeq;
    call('recent_inputs', {}).then(function (list) {
      if (seq === ST.recentSeq) fillRecent(Array.isArray(list) ? list : []);
    }, function () {
      if (seq === ST.recentSeq) fillRecent(WT.recent || []);
    });
  }
  // Rebuilt only when the list changed; the item picked with the arrows stays picked.
  function fillRecent(list) {
    var x = fieldParts(), next = list.slice(0, 10);
    if (!x) return;
    if (next.join('\n') !== ST.recent.join('\n') || !x.list.children.length) {
      var vis = visibleRecent(), keepHost = ST.active >= 0 && vis[ST.active] ? vis[ST.active].dataset.host : null;
      ST.recent = next;
      x.list.innerHTML = recentItems(next);
      ST.active = -1;
      showRecent();
      if (keepHost) setActive(visibleRecent().map(function (it) { return it.dataset.host; }).indexOf(keepHost));
    } else showRecent();
  }
  function visibleRecent() { var x = fieldParts(); return x ? $$('[data-host]:not([hidden])', x.list) : []; }
  function showRecent(want) {
    if (want != null) ST.wantRecent = want;
    var x = fieldParts();
    if (!x) return;
    var v = x.input.value.trim().toLowerCase(), any = false;
    $$('[data-host]', x.list).forEach(function (it) {
      var ok = !v || it.dataset.host.toLowerCase().indexOf(v) >= 0;
      it.hidden = !ok; any = any || ok;
    });
    var vis = ST.wantRecent && any && document.activeElement === x.input && !x.fld.classList.contains('is-error');
    x.menu.hidden = !vis;
    x.input.setAttribute('aria-expanded', String(vis));
    setActive(vis ? Math.min(ST.active, visibleRecent().length - 1) : -1);
  }
  function setActive(k) {
    var x = fieldParts(), vis = visibleRecent();
    ST.active = k;
    $$('[data-host]', x.list).forEach(function (it) { it.classList.remove('is-hover'); it.setAttribute('aria-selected', 'false'); });
    if (k >= 0 && vis[k]) {
      vis[k].classList.add('is-hover'); vis[k].setAttribute('aria-selected', 'true');
      x.input.setAttribute('aria-activedescendant', vis[k].id);
    } else x.input.removeAttribute('aria-activedescendant');
  }
  function pickRecent(host) {
    var x = fieldParts();
    if (!x) return;
    x.input.value = host;
    clearError();
    showRecent(false);
    x.input.focus();
  }

  // Adds a target with the probe types and the interval picked in the header, one row per probe type; errors show
  // under the field. host: a name or IP (the field's text by default; WTH.add(host) for the first-run quick start).
  // Resolves { id, ids, host, probes, intervalS } once a row is added (with error: the code, when a later probe
  // type failed); rejects with the error code when nothing was added. An add on its way returns its own promise.
  function add(host) {
    if (ST.adding) return ST.adding;
    var x = fieldParts();
    host = String(host == null ? (x ? x.input.value : '') : host).trim();
    if (!host) { showError('empty'); if (x) x.input.focus(); return Promise.reject('empty'); }
    var probes = selected().map(probeId).filter(Boolean), iv = ST.interval, ids = [], cur = probes[0];
    if (!probes.length) { showError('probe_unavailable', host); return Promise.reject('probe_unavailable'); }
    busy(true);
    // One row per probe type: add them one after another.
    ST.adding = probes.reduce(function (q, probe) {
      return q.then(function () {
        cur = probe;
        return call('add_target', { req: { host: host, probe: probe, intervalS: iv } }).then(function (r) { ids.push(r && r.id); });
      });
    }, Promise.resolve()).then(function () { return finish(null); }, finish);
    return ST.adding;
    function finish(err) {
      ST.adding = null;
      busy(false);
      var res = { id: ids[0], ids: ids, host: host, probes: probes, intervalS: iv };
      if (!err) {
        var y = fieldParts();
        if (y && y.input.value.trim() === host) y.input.value = '';
        clearError();
        showRecent(false);
      }
      if (err) { showError(err, host, cur); res.error = codeOf(err); }
      if (ids.length && CB.onAdded) { try { CB.onAdded(res); } catch (x) { console.error(x); } }
      if (err && !ids.length) throw err;
      return res;
    }
  }
  function busy(on) { var b = $('.wth-add-btn'); if (b) { if (on) b.setAttribute('aria-busy', 'true'); else b.removeAttribute('aria-busy'); } }

  function toggleProbe(b) {
    if (b.getAttribute('aria-disabled') === 'true') return;
    var p = b.dataset.probe, i = ST.probes.indexOf(p);
    if (i >= 0) { if (ST.probes.length === 1) return; ST.probes.splice(i, 1); } // at least one type stays on
    else ST.probes.push(p);
    syncProbes();
  }
  // In place, so focus and hover stay; a button is rebuilt only when its type becomes available or goes away.
  function syncProbes() {
    var box = $('.wth-probes');
    if (!box) return;
    selected();
    $$('[data-probe]', box).forEach(function (b) {
      var p = PROBES.filter(function (x) { return x[0] === b.dataset.probe; })[0];
      if (!p) return;
      var off = !probeId(p[0]), on = !off && ST.probes.indexOf(p[0]) >= 0;
      if (off !== (b.getAttribute('aria-disabled') === 'true')) { b.outerHTML = probeBtn(p); return; }
      if (b.getAttribute('aria-pressed') !== String(on)) {
        b.setAttribute('aria-pressed', String(on));
        var svg = $('svg', b);
        if (svg) svg.outerHTML = I(on ? 'check' : 'plus');
      }
    });
    var port = $('.wth-port');
    if (port) port.hidden = ST.probes.indexOf('tcp') < 0;
  }

  function ivParts() { var s = $('.wth-select'); return s && { sel: s, btn: $('.wth-select-btn', s), menu: $('.wth-menu', s) }; }
  function ivOpen() { var x = ivParts(); return !!x && x.sel.classList.contains('is-open'); }
  // byKey: opened from the keyboard, the current interval is highlighted for the arrows.
  function openInterval(open, byKey) {
    var x = ivParts();
    if (!x) return;
    x.sel.classList.toggle('is-open', open); x.menu.hidden = !open; x.btn.setAttribute('aria-expanded', String(open));
    if (open) showRecent(false);
    ST.ivActive = open && byKey ? WT.intervals.indexOf(ST.interval) : -1;
    markIv();
  }
  function markIv() {
    var x = ivParts();
    if (x) $$('[data-iv]', x.menu).forEach(function (it, k) { it.classList.toggle('is-hover', k === ST.ivActive); });
  }
  function pickInterval(n) {
    var x = ivParts();
    if (!x || WT.intervals.indexOf(n) < 0) return;
    var was = ST.interval;
    ST.interval = n;
    var s = secs(n), val = $('.wth-iv-val', x.sel), lb = ivLabel(n);
    val.textContent = L(s.ru, s.en); val.setAttribute('data-en', s.en);
    x.btn.setAttribute('aria-label', L(lb.ru, lb.en)); x.btn.setAttribute('data-en-aria-label', lb.en);
    $$('[data-iv]', x.menu).forEach(function (it) {
      var on = parseFloat(it.dataset.iv) === n;
      it.classList.toggle('is-on', on); it.setAttribute('aria-selected', String(on));
      $('.ck', it).innerHTML = on ? I('check') : '';
    });
    openInterval(false);
    if (n !== was && CB.onInterval) CB.onInterval(n);
  }

  function markWindow(key) { $$('.wth [data-win]').forEach(function (b) { b.setAttribute('aria-pressed', String(b.dataset.win === key)); }); }
  function pickWindow(key) {
    if (!WT.windowSeconds[key]) return;
    var was = curWindow();
    markWindow(key);
    if (key === was) return;
    if (CB.onWindow) CB.onWindow(key);
    else if (window.Live) window.Live.set({ window: key }).then(null, function () { /* logged by Live */ });
  }

  // ---------- Settings ----------
  function panel() { return $('.wth-side'); }
  function openSettings(open) {
    var p = panel(), g = $('.wth-gear');
    if (!p) return;
    if (open == null) open = p.hidden;
    p.hidden = !open;
    if (g) g.setAttribute('aria-expanded', String(open));
    if (open) { openInterval(false); syncPanel(); }
  }

  // Saved settings come back from the core; the legend and the card follow, the screen hears onSettings.
  function applied(s, patch) {
    WT.settings = Object.assign({}, WT.settings, s && typeof s === 'object' ? s : patch);
    syncLegend(); syncPanel();
    if (CB.onSettings) CB.onSettings(WT.settings, patch);
    return WT.settings;
  }
  function failed(err) { console.error('set_settings failed', err); return null; }
  function saveSettings(patch) {
    return call('set_settings', { settings: patch }).then(function (s) { return applied(s, patch); }, failed);
  }

  function setTheme(v) {
    if (['system', 'light', 'dark'].indexOf(v) < 0 || (v === curTheme() && v === settings().theme)) return;
    markPressed('data-theme-set', v);
    var patch = { theme: v };
    if (window.WTTheme) { window.WTTheme.set(patch).then(function (s) { return applied(s, patch); }, failed); return; }
    if (v === 'system') delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = v;
    saveSettings(patch);
  }

  // The language applies through a reload: every part renders its texts once. The card stays open after it.
  function setLang(v) {
    v = v === 'en' ? 'en' : 'ru';
    if (v === curLang()) { if (settings().lang !== v) saveSettings({ lang: v }); return; }
    markPressed('data-lang-set', v);
    keepOpenOnReload();
    if (window.WTTheme) { window.WTTheme.set({ lang: v }).then(null, failed); return; }
    var go = function () {
      var q = new URLSearchParams(location.search);
      if (v === 'en') q.set('lang', 'en'); else q.delete('lang');
      var s = q.toString();
      location.replace(location.pathname + (s ? '?' + s : '') + location.hash);
    };
    saveSettings({ lang: v }).then(go, go);
  }
  function markPressed(attr, v) { $$('[' + attr + ']').forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute(attr) === v)); }); }
  // An open card opens again after the language reload (per tab, see bindAll).
  function keepOpenOnReload() {
    var p = panel();
    try { if (p && !p.hidden) sessionStorage.setItem('wth-settings-open', '1'); } catch (e) { /* no storage */ }
  }

  // Thresholds are saved a moment after typing stops; the legend follows at once.
  function readTh(p) {
    var wi = $('[data-th="warn"]', p), bi = $('[data-th="bad"]', p);
    return { wi: wi, bi: bi, warn: parseInt(wi.value, 10), bad: parseInt(bi.value, 10) };
  }
  function validTh(t) { return t.warn >= 1 && t.bad > t.warn && t.bad < 10000; }
  function onThInput(p, input) {
    var clean = input.value.replace(/\D/g, '');
    if (clean !== input.value) input.value = clean;
    var t = readTh(p);
    clearTimeout(ST.thTimer); ST.thTimer = null;
    if (!validTh(t)) return;
    ST.thLocal = { warn: t.warn, bad: t.bad };
    syncLegend();
    ST.thTimer = setTimeout(function () {
      ST.thTimer = null;
      saveSettings({ warnMs: t.warn, badMs: t.bad }).then(function () {
        // values typed while this was saving stay until their own save
        if (ST.thLocal && ST.thLocal.warn === t.warn && ST.thLocal.bad === t.bad) ST.thLocal = null;
        syncLegend(); syncPanel();
      });
    }, 400);
  }
  // Leaving a field with a value that does not fit (empty, second not above first) brings back the saved one.
  function onThChange(p) {
    var t = readTh(p);
    if (validTh(t)) return;
    clearTimeout(ST.thTimer); ST.thTimer = null; ST.thLocal = null;
    var th = thresholds();
    t.wi.value = th.warn; t.bi.value = th.bad;
    syncLegend();
  }

  // ---------- Live update: in place, so focus, open menus and hover stay ----------
  function syncLegend() {
    var th = thresholds();
    $$('.wth-legend').forEach(function (ul) {
      if (ul.dataset.warn !== String(th.warn) || ul.dataset.bad !== String(th.bad)) ul.outerHTML = legend();
    });
  }
  function syncPanel() {
    var p = panel();
    if (!p) return;
    if (!ST.thLocal && !ST.thTimer) {
      var th = thresholds();
      $$('[data-th]', p).forEach(function (i) {
        var v = String(i.dataset.th === 'warn' ? th.warn : th.bad);
        if (document.activeElement !== i && i.value !== v) i.value = v;
      });
    }
    markPressed('data-theme-set', curTheme());
    markPressed('data-lang-set', curLang());
  }
  function update() {
    if (!$('.wth') && !panel()) return;
    markWindow(curWindow());
    syncProbes();
    syncLegend();
    syncPanel();
  }

  // ---------- Events (delegated, so a re-rendered header or card keeps working) ----------
  function inParts(el) { return !!(el && el.closest && el.closest('.wth, .wth-side')); }

  function onClick(e) {
    var t = e.target, b;
    if (!t.closest) return;
    if (!t.closest('.wth-select')) { if (ivOpen()) openInterval(false); }
    if (!inParts(t)) return;
    if ((b = t.closest('.wth [data-win]'))) pickWindow(b.dataset.win);
    else if ((b = t.closest('.wth [data-probe]'))) toggleProbe(b);
    else if ((b = t.closest('.wth-select-btn'))) {
      // Enter or Space on the open list picks the highlighted interval (keyboard clicks have detail 0)
      if (ivOpen() && e.detail === 0 && ST.ivActive >= 0) pickInterval(WT.intervals[ST.ivActive]);
      else openInterval(!ivOpen(), e.detail === 0);
    } else if ((b = t.closest('[data-iv]'))) pickInterval(parseFloat(b.dataset.iv));
    else if ((b = t.closest('.wth-gear'))) openSettings();
    else if ((b = t.closest('.wth-side .wth-close'))) { openSettings(false); var g = $('.wth-gear'); if (g) g.focus(); }
    // A screen that handles theme and language itself stops the click before this listener: then only the marks follow
    else if ((b = t.closest('[data-lang-set]'))) {
      if (!e.cancelBubble) setLang(b.dataset.langSet);
      else if (b.dataset.langSet !== curLang()) { markPressed('data-lang-set', b.dataset.langSet); keepOpenOnReload(); }
    }
    else if ((b = t.closest('[data-theme-set]'))) { if (e.cancelBubble) markPressed('data-theme-set', b.dataset.themeSet); else setTheme(b.dataset.themeSet); }
    else if ((b = t.closest('.wth-add-btn'))) add().then(null, noop); // the error is under the field
  }

  function onMouseDown(e) {
    var t = e.target, it;
    if (!t.closest) return;
    if ((it = t.closest('.wth-history [data-host]'))) { e.preventDefault(); pickRecent(it.dataset.host); }
    else if (t.closest('.wth-history, .wth-intervals')) e.preventDefault(); // menus keep the focus where it is
    else if (t.classList.contains('wth-input') && document.activeElement === t && $('.wth-history').hidden) openRecent();
  }

  function onInput(e) {
    var t = e.target;
    if (!t.classList) return;
    if (t.classList.contains('wth-input')) { clearError(); ST.active = -1; showRecent(true); return; }
    var p = t.closest && t.closest('.wth-side');
    if (p && t.matches('[data-th]')) onThInput(p, t);
  }
  function onChange(e) {
    var t = e.target, p = t.closest && t.closest('.wth-side');
    if (p && t.matches('[data-th]')) onThChange(p);
  }

  // The recent list opens when the user puts the caret in the field (click, Tab), not when a screen focuses it.
  function onFocusIn(e) {
    var t = e.target;
    if (!t.classList || !t.classList.contains('wth-input')) return;
    if (ivOpen()) openInterval(false);
    var user = !ST.silentFocus && Date.now() - ST.lastInput < 500;
    ST.silentFocus = false;
    if (user) openRecent(); else loadRecent();
  }
  function onFocusOut(e) {
    var t = e.target;
    if (t.classList && t.classList.contains('wth-input')) showRecent(false);
  }

  // Esc goes to what is on top first: a dialog over the screen, or a drag or a period being picked (their parts
  // cancel it); the header leaves it to them.
  function escElsewhere() {
    var h = document.documentElement;
    if (ST.pointerDown || h.classList.contains('tl-dragging') || h.classList.contains('sum-dragging')) return true;
    return $$('[aria-modal="true"]').some(function (m) { return m.getClientRects().length > 0; });
  }

  // Capture phase: Esc closes only what is open in the header or the card and goes no further
  // (a period picked on a timeline is dropped by the next press).
  function onKey(e) {
    var t = e.target, x, vis, k;
    ST.lastInput = Date.now();
    if (e.key === 'Escape') {
      if (e.defaultPrevented || escElsewhere()) return;
      var p = panel(), done = true;
      if (ivOpen()) { var inIv = t.closest && t.closest('.wth-select'); openInterval(false); if (inIv) ivParts().btn.focus(); }
      else if ((x = fieldParts()) && !x.menu.hidden) showRecent(false);
      else if (p && !p.hidden) { var inside = p.contains(document.activeElement); openSettings(false); var g = $('.wth-gear'); if (g && inside) g.focus(); }
      else done = false;
      if (done) { e.preventDefault(); e.stopPropagation(); }
      return;
    }
    if (!t.classList || e.isComposing) return;
    if (t.classList.contains('wth-input')) {
      x = fieldParts();
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (x.menu.hidden) { ST.active = -1; openRecent(); }
        vis = visibleRecent();
        if (x.menu.hidden || !vis.length) return;
        k = ST.active + (e.key === 'ArrowDown' ? 1 : -1);
        setActive(k < 0 ? vis.length - 1 : k >= vis.length ? 0 : k);
      } else if (e.key === 'Enter') {
        e.preventDefault();
        vis = visibleRecent();
        if (!x.menu.hidden && ST.active >= 0 && vis[ST.active]) pickRecent(vis[ST.active].dataset.host);
        else add().then(null, noop);
      } else if (e.key === 'Tab') showRecent(false);
      return;
    }
    if (t.classList.contains('wth-select-btn') && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault();
      if (!ivOpen()) { openInterval(true, true); return; }
      var n = WT.intervals.length, from = ST.ivActive >= 0 ? ST.ivActive : WT.intervals.indexOf(ST.interval);
      k = from + (e.key === 'ArrowDown' ? 1 : -1);
      ST.ivActive = k < 0 ? n - 1 : k >= n ? 0 : k;
      markIv();
    } else if (t.classList.contains('wth-select-btn') && e.key === 'Tab' && ivOpen()) openInterval(false);
  }

  var inited = false;
  function init() {
    if (inited) return;
    inited = true;
    document.addEventListener('click', onClick);
    document.addEventListener('mousedown', onMouseDown);
    document.addEventListener('input', onInput, true); // capture: other listeners read the cleaned threshold
    document.addEventListener('change', onChange);
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('focusout', onFocusOut);
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('pointerdown', function (e) { ST.lastInput = Date.now(); ST.pointerDown = !inParts(e.target); }, true);
    var up = function () { ST.pointerDown = false; };
    document.addEventListener('pointerup', up, true);
    document.addEventListener('pointercancel', up, true);
    document.addEventListener('pointermove', function (e) { if (ST.pointerDown && !e.buttons) up(); }, true);
    window.addEventListener('blur', up);
    document.addEventListener('submit', function (e) { if (e.target.closest && e.target.closest('.wth-add')) e.preventDefault(); });
    if (window.Live && window.Live.onData) window.Live.onData(update);
  }

  // Wires the header and the settings card that are on the page now; safe to call again after a re-render.
  // o: the same callbacks as header(o).
  function bindAll(o) {
    keep(o);
    init();
    update();
    var reopen = false;
    try { reopen = sessionStorage.getItem('wth-settings-open') === '1'; sessionStorage.removeItem('wth-settings-open'); } catch (e) { /* no storage */ }
    if (reopen) openSettings(true);
    if (ST.focusOnBind || (o && o.focus)) { ST.focusOnBind = false; focusField(); }
  }

  // The screen's thresholds (from the data): the legend and the card follow unless the card is being edited.
  function setThresholds(warn, bad) {
    if (!(warn >= 1 && bad > warn && bad < 10000)) return false;
    if (ST.thLocal || ST.thTimer) return true;
    var s = settings();
    if (+s.warnMs !== warn || +s.badMs !== bad) WT.settings = Object.assign({}, s, { warnMs: warn, badMs: bad });
    syncLegend(); syncPanel();
    return true;
  }

  // Caret in the field without opening the recent list (first run).
  function focusField() {
    var x = fieldParts();
    if (!x || document.activeElement === x.input) return;
    ST.silentFocus = true;
    x.input.focus();
    ST.silentFocus = false;
  }

  window.WTH = {
    header: header, settingsPanel: settingsPanel, legend: legend, bindAll: bindAll, update: update,
    openSettings: openSettings, focusField: focusField, setThresholds: setThresholds, T: T, L: L, lang: curLang(),
    // The first-run quick start adds through the header: WTH.add(host) is a promise (see add()).
    add: function (host) { return add(host); },
    interval: function () { return ST.interval; },
  };
})();
