// Theme and language of the window. The saved settings decide (Settings: theme, lang). They live in the core,
// and a copy is kept in the page's storage so the page opens in the right theme and language at once, before the
// first snapshot: localStorage in the app, sessionStorage in a plain browser (every tab keeps its own, like the
// mock's settings). ?theme=light|dark and ?lang=ru|en override them for checks.
// Texts marked with data-en switch to English on load; parts read the language once, so a new language reloads the page.
(function () {
  var KEY = 'wt-ui', root = document.documentElement, q = new URLSearchParams(location.search);
  var store = null;
  try { store = window.__TAURI__ ? window.localStorage : window.sessionStorage; } catch (e) { /* no storage */ }
  var saved = read();
  var forcedTheme = /^(light|dark)$/.test(q.get('theme') || '') ? q.get('theme') : null;
  var forcedLang = /^(ru|en)$/.test(q.get('lang') || '') ? q.get('lang') : null;
  var lang = forcedLang || (saved.lang === 'en' ? 'en' : 'ru');

  function read() { try { return JSON.parse(store.getItem(KEY) || '{}') || {}; } catch (e) { return {}; } }
  // Remembers theme and language; true when the copy can be read back (storage may be off).
  function remember(theme, lng) {
    try {
      store.setItem(KEY, JSON.stringify({ theme: theme, lang: lng }));
      return read().lang === lng;
    } catch (e) { return false; }
  }
  function setTheme(t) {
    if (t === 'light' || t === 'dark') root.dataset.theme = t;
    else delete root.dataset.theme; // system
    paintWindow();
  }
  // In the app the window behind the page takes the page's background, so its edges stay in the theme while
  // the window grows. Asked only when the shown theme changes.
  var painted = null, dark = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  function paintWindow() {
    if (!window.__TAURI__) return;
    var t = root.dataset.theme, isDark = t ? t === 'dark' : !!(dark && dark.matches);
    if (isDark === painted) return;
    painted = isDark;
    try { window.__TAURI__.core.invoke('set_background', { dark: isDark }).catch(function () {}); } catch (e) { /* no core */ }
  }
  if (dark && dark.addEventListener) dark.addEventListener('change', function () { if (!root.dataset.theme) paintWindow(); });
  function langOf(s) { return s && s.lang === 'en' ? 'en' : 'ru'; }
  // Reload in another language. Without storage the language goes into the address, so the page cannot loop.
  function relaunch(lng, theme) {
    var u = new URL(location.href);
    if (remember(theme, lng)) u.searchParams.delete('lang'); else u.searchParams.set('lang', lng);
    location.replace(u.href);
  }

  setTheme(forcedTheme || saved.theme);
  root.lang = lang;
  root.dataset.lang = lang;
  if (lang === 'en') {
    document.addEventListener('DOMContentLoaded', function () {
      document.querySelectorAll('[data-en]').forEach(function (el) { el.textContent = el.getAttribute('data-en'); });
    });
  }

  window.WTTheme = {
    lang: lang,
    // Current theme setting: system, light or dark.
    theme: function () { return root.dataset.theme || 'system'; },
    // Saved settings from the core (base/live.js calls it with every snapshot): the theme applies at once,
    // a different language reloads the page.
    sync: function (s) {
      if (!s) return;
      var theme = s.theme || 'system', lng = langOf(s);
      if (saved.theme !== theme || saved.lang !== lng) { remember(theme, lng); saved = { theme: theme, lang: lng }; }
      if (!forcedTheme) setTheme(theme);
      if (!forcedLang && lng !== lang) relaunch(lng, theme);
    },
    // A choice in Settings ({theme} or {lang}): saved in the core, the theme applies at once,
    // a new language reloads the page once the core has it. The ?theme / ?lang override gives way to the choice.
    set: function (patch) {
      var done = window.API ? window.API.call('set_settings', { settings: patch }) : Promise.resolve();
      if ('theme' in patch) {
        forcedTheme = null;
        setTheme(patch.theme);
        saved = { theme: patch.theme || 'system', lang: saved.lang || lang };
        remember(saved.theme, saved.lang);
        if (q.has('theme')) {
          var u = new URL(location.href);
          u.searchParams.delete('theme');
          history.replaceState(history.state, '', u.href);
        }
      }
      if ('lang' in patch && langOf(patch) !== lang) {
        var go = function () { relaunch(langOf(patch), saved.theme || window.WTTheme.theme()); };
        return done.then(go, go);
      }
      return done;
    },
  };
})();
