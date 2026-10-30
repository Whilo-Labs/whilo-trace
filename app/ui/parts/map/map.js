// Whilo Trace, part "route map" (MAP): the «Карта» view of the hop card on the trace screen, a port of the approved
// mockup C «Телеметрия» (Дизайн/Whilo Trace карта маршрута). The map well on the left (canvases), the side column on
// the right: the selected hop with its speed-of-light check, then the hop ribbon. One map per page.
// Data: what screens/trace.js passes to MAP.update on every redraw of the trace part: the part's model (m: hops with
// status, origin, stats s (window or picked period), full (window) and geo, selected, probe, win, range, rounds), the
// trace's home and WT.geo (docs/ui-contract.md). Where every hop goes is worked out by parts/map/map-geo.js (MAPG).
// Layers: a hidden cache (land, grid, edge ticks, crosses); L0 (cache, route, markers, reticle); L1 (packet and its
// readout: the only layer meant to be redrawn per frame); L2 (callouts, legend, scale and sources, edge pointer, degree
// labels). A layer is redrawn only when what it shows changed. Colours are tokens read from the page (again when the
// theme changes; the card in full screen is dark). Actions leave through callbacks: onSelect(ttl), onHome({lat, lon} |
// null: «Дом здесь» / «Сбросить дом» in the right-click menu), onPause(paused: the system's reduced-motion setting
// changed while the page is open).
// The packet follows the target's real rounds (MAP.round, the "round" event): one packet per round with the outcome
// the next snapshot shows, at most 30 frames a second, and no frame loop at all while nothing moves.
// MAP.png draws the picture for publication (dark, your network hidden) and the report's map (light, as it is);
// parts/map/map-picture.js puts it in the «Сохранить картинку» dialog (MAP.picture).
// MAP: init(o), attach(host | null), update(data), round(targetId), setPaused(on), paused(), renderAt(t, lostAt),
// png(data, o), hasPlaces(data).
// Test hooks: window.MAP_READY = true after the first complete draw; MAP.renderAt(t) draws the packet t seconds into
// a round of the current data (lostAt: the ttl where that round is lost) and freezes that frame and the live
// animation, MAP.renderAt(null) the static route.
(function () {
  'use strict';
  var EN = document.documentElement.dataset.lang === 'en';
  function L(ru, en) { return EN ? en : ru; }
  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; });
  }
  function fin(v) { return v != null && isFinite(v); }
  var G = window.MAPG, RING = G.RING, NB = ' ';
  var MONO = '"Cascadia Mono", ui-monospace, Consolas, Menlo, "DejaVu Sans Mono", monospace';
  var SYS = '"Segoe UI", system-ui, -apple-system, sans-serif';
  function mq(q) { return window.matchMedia ? window.matchMedia(q) : null; }
  function onMq(m, fn, on) {
    if (!m) return;
    if (m.addEventListener) { if (on === false) m.removeEventListener('change', fn); else m.addEventListener('change', fn); }
    else if (m.addListener) { if (on === false) m.removeListener(fn); else m.addListener(fn); }
  }
  var CALM = mq('(prefers-reduced-motion: reduce)'), DARK = mq('(prefers-color-scheme: dark)');

  // ---------- numbers: the table's formatter (WT.fmt) for RTT and loss, one decimal in the physics block ----------
  // Half up from the decimal value the core sent (75.55 is 75.5499... in binary: toFixed alone gives 75.5).
  function dec(v, d) {
    var dd = d == null ? 1 : d, k = Math.pow(10, dd), s = (Math.round(+v * k + 1e-6) / k).toFixed(dd);
    return EN ? s : s.replace('.', ',');
  }
  function nf(v) { if (v === 0) return '0'; var s = WT.fmt(v); return EN ? s.replace(',', '.') : s; }
  function thou(n) {
    var s = String(Math.round(Math.abs(n))), out = '';
    while (s.length > 3) { out = (EN ? ',' : NB) + s.slice(-3) + out; s = s.slice(0, -3); }
    return (n < 0 ? '-' : '') + s + out;
  }
  function msT(v) { return nf(v) + NB + L('мс', 'ms'); }
  function ms1(v) { return dec(v) + NB + L('мс', 'ms'); }
  function kmT(v) { return thou(v) + NB + L('км', 'km'); }
  function pctT(v) { var s = v === 0 ? '0' : nf(v); return EN ? s + '%' : s + NB + '%'; }
  function ns(v) { return v >= 0 ? L('с.' + NB + 'ш.', 'N') : L('ю.' + NB + 'ш.', 'S'); }
  function ew(v) { return v >= 0 ? L('в.' + NB + 'д.', 'E') : L('з.' + NB + 'д.', 'W'); }
  function latT(v, d) { return dec(Math.abs(v), d) + '°' + NB + ns(v); }
  function lonT(v, d) { return dec(Math.abs(v), d) + '°' + NB + ew(v); }
  function degLat(v) { return v === 0 ? '0°' : Math.abs(v) + '°' + NB + ns(v); }
  function degLon(v) { v = G.norm(v); return v === 0 || Math.abs(v) === 180 ? Math.abs(v) + '°' : Math.abs(v) + '°' + NB + ew(v); }
  // The next "nice" number up: 1, 2, 2.5, 5 x 10^k (fine: also 1.2, 1.5, 3, 4, 6, 8, for a scale maximum).
  function niceUp(v, fine) {
    if (!(v > 0)) return 1;
    var p = Math.pow(10, Math.floor(Math.log(v) / Math.LN10)), f = v / p, steps = fine ? [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10] : [1, 2, 2.5, 5, 10];
    for (var i = 0; i < steps.length; i++) if (f <= steps[i] + 1e-9) return steps[i] * p;
    return 10 * p;
  }

  // ---------- texts ----------
  function probeName(m) { var p = WT.probes[m.probe]; return p ? L(p.ru, p.en) : String(m.probe || '').toUpperCase(); }
  function stWord(key, m) { var s = WT.statuses[key] || WT.statuses.noreply; return L(s.ru, s.en).replace('{probe}', probeName(m)); }
  function stOf(h, m) { return h.origin ? L(WT.statuses.origin.ru, WT.statuses.origin.en) : stWord(h.status, m); }
  function winOf(m) { var w = WT.windows.filter(function (x) { return x[0] === m.win; })[0]; return w ? L(w[1], w[2]) : ''; }
  function nameOf(h) { return h.name === 'роутер' ? L('роутер', 'router') : h.name || ''; }
  function placeName(g) { var n = g.city || g.country || g.cc || ''; return g.cc && n !== g.cc ? n + ', ' + g.cc : n; }
  var MON = [['янв.', 'Jan'], ['февр.', 'Feb'], ['март', 'Mar'], ['апр.', 'Apr'], ['май', 'May'], ['июнь', 'Jun'], ['июль', 'Jul'],
    ['авг.', 'Aug'], ['сент.', 'Sep'], ['окт.', 'Oct'], ['нояб.', 'Nov'], ['дек.', 'Dec']];
  function monthShort(s) { var x = /^(\d{4})-(\d{2})$/.exec(s || ''), k = x ? +x[2] - 1 : -1; return MON[k] ? L(MON[k][0], MON[k][1]) + ' ' + x[1] : ''; }
  var SPECIAL = {
    private: ['частный диапазон', 'private range'], cgnat: ['общий диапазон CGNAT', 'shared CGNAT range'], loopback: ['loopback', 'loopback'],
    'link-local': ['локальный канал', 'link-local'], documentation: ['документация', 'documentation'], benchmarking: ['тестирование сетей', 'benchmarking'],
    multicast: ['multicast', 'multicast'], reserved: ['резерв', 'reserved'], unspecified: ['адрес не задан', 'unspecified'], broadcast: ['широковещательный', 'broadcast'],
  };
  function specialName(k) { var s = SPECIAL[k]; return s ? L(s[0], s[1]) : String(k || ''); }

  // ---------- state ----------
  var opt = {}, stage = null, host = null, mapEl = null, sideEl = null, panelEl = null, ribEl = null, emptyEl = null, menuEl = null;
  var cv = [], cx = [], cache = null, cacheCx = null, landF = null;
  var W = 0, H = 0, DPR = 1, C = {}, colorKey = '';
  var input = null, sc = null, fitPrev = null, choicePrev = {}, hits = [];
  var keys = { size: '', cache: '', l0: '', l2: '', l1: '', panel: '', sel: null, empty: null };
  var paused = !!(CALM && CALM.matches), calm = paused, frozen = null, hold = false, demoLost, cursor = { ttl: null, lost: false };
  var ro = null, dprMq = null, raf = 0, menuLL = null, ready = false, lastD = null, XP = false;

  // ---------- canvas helpers ----------
  function font(c, px, fam, weight) { c.font = (weight ? weight + ' ' : '') + px + 'px ' + fam; }
  var PF = { sys: [12, SYS, 400], sysB: [12, SYS, 600], mono: [12, MONO, 400], monoS: [11, MONO, 400] };
  function partFont(c, p) { var f = PF[p.f || 'sys']; font(c, f[0], f[1], f[2]); }
  function partW(c, p) { partFont(c, p); return c.measureText(p.t).width; }
  function hitB(a, b) { return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h; }
  var TOK = ['well', 'layer', 'grid', 'faint', 'muted', 'title', 'text', 'line', 'loss', 'loss-text', 'partial', 'partial-text', 'hatch', 'packet', 'mp-on-line'];
  function readColors() {
    var cs = getComputedStyle(stage), o = {}, key = '';
    TOK.forEach(function (k) { o[k] = cs.getPropertyValue('--' + k).trim(); key += o[k] + ';'; });
    C = o;
    return key;
  }
  function land() {
    if (!landF && window.WT_LAND_110M && window.topojson) landF = topojson.feature(WT_LAND_110M, WT_LAND_110M.objects.land);
    return landF || { type: 'FeatureCollection', features: [] };
  }

  // ---------- markers (shape = how sure the place is) ----------
  function mCity(c, x, y) { c.fillStyle = C.title; c.fillRect(Math.round(x - 3.5), Math.round(y - 3.5), 7, 7); }
  function mHome(c, x, y) {
    c.fillStyle = C.title; c.beginPath();
    c.moveTo(x, y - 7); c.lineTo(x + 6.5, y - 1.5); c.lineTo(x + 6.5, y + 6); c.lineTo(x - 6.5, y + 6); c.lineTo(x - 6.5, y - 1.5);
    c.closePath(); c.fill();
  }
  function qMark(c, x, y) {
    font(c, 11, MONO, 700); c.fillStyle = C.title; c.textAlign = 'center'; c.textBaseline = 'middle';
    c.fillText('?', x, y + 0.5); c.textAlign = 'left';
  }
  function mUnknown(c, x, y, r) {
    c.fillStyle = C.well; c.strokeStyle = C.title; c.lineWidth = 1.5;
    c.beginPath(); c.arc(x, y, r || 8, 0, Math.PI * 2); c.fill(); c.stroke(); qMark(c, x, y);
  }
  function mImpossible(c, x, y, s) {
    s = s || 15;
    c.fillStyle = C.well; c.fillRect(x - s / 2, y - s / 2, s, s);
    c.strokeStyle = C.title; c.lineWidth = 1.5; c.setLineDash([3, 2]);
    c.strokeRect(x - s / 2 + 0.75, y - s / 2 + 0.75, s - 1.5, s - 1.5); c.setLineDash([]); qMark(c, x, y);
  }
  function mCountry(c, x, y, r) {
    c.strokeStyle = C.title; c.lineWidth = 1.5; c.setLineDash([3.5, 3]);
    c.beginPath(); c.arc(x, y, r || RING, 0, Math.PI * 2); c.stroke(); c.setLineDash([]);
  }
  function marker(c, kind, x, y) {
    if (kind === 'home' || kind === 'own') mHome(c, x, y);
    else if (kind === 'unknown') mUnknown(c, x, y);
    else if (kind === 'impossible') mImpossible(c, x, y);
    else if (kind === 'country') mCountry(c, x, y);
    else mCity(c, x, y);
  }
  // Brackets around the selected hop's marker, clear of it: a country ring is wider than a city square.
  function retS(mk) { return Math.max(12, (MR[mk] || 4) + 8); }
  function reticle(c, x, y, mk) {
    var s = retS(mk), arm = 5;
    c.strokeStyle = C.title; c.lineWidth = 1.5; c.beginPath();
    [[-1, -1], [1, -1], [1, 1], [-1, 1]].forEach(function (k) {
      var cxp = x + k[0] * s, cyp = y + k[1] * s;
      c.moveTo(cxp - k[0] * arm, cyp); c.lineTo(cxp, cyp); c.lineTo(cxp, cyp - k[1] * arm);
    });
    c.stroke();
  }
  // Marker size for the callouts (obstacle box) and where a leader starts.
  var MR = { home: 7, own: 7, city: 4, country: RING, unknown: 8, impossible: 8, anchored: 4 };
  var R0 = { home: 9, own: 9, city: 7, country: RING + 2, unknown: 10, impossible: 12, anchored: 7 };

  // Status strip under a TTL badge (shape, not only colour): solid bar = loss, three ticks = answers some probes,
  // hatching = does not answer this probe type, a grey square = no reply / says the destination is unreachable.
  function strip(c, x, y, w, status, hgt) {
    hgt = hgt || 4;
    if (status === 'loss') { c.fillStyle = C.loss; c.fillRect(x, y, w, hgt); }
    else if (status === 'partial') {
      c.fillStyle = C.partial;
      var o = Math.round((w - 15) / 2);
      for (var i = 0; i < 3; i++) c.fillRect(x + o + i * 6, y, 3, hgt);
    } else if (status === 'silent') {
      c.save(); c.beginPath(); c.rect(x, y, w, hgt); c.clip();
      c.strokeStyle = C.hatch; c.lineWidth = 1.5; c.beginPath();
      for (var k = -hgt; k < w + hgt; k += 4) { c.moveTo(x + k, y + hgt); c.lineTo(x + k + hgt, y); }
      c.stroke(); c.restore();
    } else if (status === 'noreply' || status === 'unreachable') { c.fillStyle = C.muted; c.fillRect(Math.round(x + w / 2 - 2), y, 4, hgt); }
  }
  function badgeW(c, ttl) { font(c, 11, MONO, 700); return Math.max(17, Math.ceil(c.measureText(String(ttl)).width) + 9); }
  function badge(c, x, y, b) {
    var w = badgeW(c, b.ttl);
    c.fillStyle = b.sel ? C.line : C.title; c.fillRect(x, y, w, 16);
    font(c, 11, MONO, 700); c.fillStyle = b.sel ? C['mp-on-line'] : C.well; c.textAlign = 'center'; c.textBaseline = 'middle';
    c.fillText(String(b.ttl), x + w / 2, y + 8.5); c.textAlign = 'left';
    strip(c, x, y + 18, w, b.status);
    return w;
  }

  // ---------- stage ----------
  function build() {
    stage = document.createElement('div');
    stage.className = 'mp-stage';
    stage.innerHTML = '<div class="mp-map" tabindex="0" role="group" aria-label="' + esc(L('Карта маршрута', 'Route map')) + '">' +
      '<canvas aria-hidden="true"></canvas><canvas aria-hidden="true"></canvas><canvas aria-hidden="true"></canvas>' +
      '<div class="mp-empty" hidden></div><div class="mp-menu" role="menu" aria-label="' + esc(L('Дом на карте', 'Home on the map')) + '" hidden></div></div>' +
      '<aside class="mp-side"><section class="mp-panel"></section><section class="mp-rib"></section></aside>';
    mapEl = stage.firstChild; sideEl = stage.lastChild;
    panelEl = sideEl.firstChild; ribEl = sideEl.lastChild;
    emptyEl = mapEl.querySelector('.mp-empty'); menuEl = mapEl.querySelector('.mp-menu');
    cv = [].slice.call(mapEl.querySelectorAll('canvas'));
    // L0 under L2 (callouts), the packet's L1 on top: a callout that has to sit on the route does not hide it
    cv.forEach(function (c, i) { c.style.zIndex = String(i === 1 ? 2 : i === 2 ? 1 : 0); });
    cache = document.createElement('canvas');
    mapEl.addEventListener('click', onMapClick);
    mapEl.addEventListener('mousemove', onMapMove);
    mapEl.addEventListener('mouseleave', function () { mapEl.style.cursor = ''; });
    mapEl.addEventListener('contextmenu', onContext);
    mapEl.addEventListener('keydown', onMapKey);
    menuEl.addEventListener('click', onMenuClick);
    menuEl.addEventListener('keydown', onMenuKey);
    ribEl.addEventListener('click', onRibClick);
    ribEl.addEventListener('keydown', onRibKey);
  }
  function resize(w, h, dpr) {
    W = w; H = h; DPR = dpr;
    cv.forEach(function (c, i) {
      c.width = Math.round(w * dpr); c.height = Math.round(h * dpr);
      c.style.width = w + 'px'; c.style.height = h + 'px';
      cx[i] = c.getContext('2d');
      cx[i].setTransform(dpr, 0, 0, dpr, 0, 0);
    });
    cache.width = Math.round(w * dpr); cache.height = Math.round(h * dpr);
    cacheCx = cache.getContext('2d');
    cacheCx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }
  function schedule() {
    if (raf || !host) return;
    raf = requestAnimationFrame(function () { raf = 0; draw(); });
  }
  // The display's pixel ratio changes when the window moves to another monitor: redraw sharp there.
  function onDpr() { watchDpr(true); schedule(); }
  function watchDpr(on) {
    if (dprMq) { onMq(dprMq, onDpr, false); dprMq = null; }
    if (on) { dprMq = mq('(resolution: ' + (window.devicePixelRatio || 1) + 'dppx)'); onMq(dprMq, onDpr); }
  }
  function observe(on) {
    if (on) {
      if (window.ResizeObserver && !ro) { ro = new ResizeObserver(schedule); ro.observe(mapEl); ro.observe(sideEl); }
      watchDpr(true);
      // The map scrolled out of view: the packet stops until it is back.
      if (window.IntersectionObserver && !an.io) {
        an.io = new IntersectionObserver(function (es) {
          var v = es[es.length - 1].isIntersecting;
          if (v === an.vis) return;
          an.vis = v;
          if (v) schedule(); else stopLoop();
        });
        an.io.observe(mapEl);
      }
    } else {
      if (ro) { ro.disconnect(); ro = null; }
      watchDpr(false);
      if (raf) { cancelAnimationFrame(raf); raf = 0; }
      if (an.io) { an.io.disconnect(); an.io = null; an.vis = true; }
    }
  }

  // ---------- data for one draw ----------
  // d: the model, hops by ttl, nodes (MAPG.kinds), keys of what each layer shows, or the reason for an empty map.
  function derive(inp) {
    var m = inp.m, hops = m.hops || [], d = { m: m, hops: hops, home: inp.home || null, geo: inp.geo || null, byTtl: {}, sel: m.selected };
    hops.forEach(function (h) { d.byTtl[h.ttl] = h; });
    d.manual = !!(d.home && d.home.source === 'manual');
    d.nodes = G.kinds(hops, d.home, window.WT_COUNTRY_POINTS);
    if (!d.nodes.some(function (n) { return n.ll; })) { d.empty = emptyWhy(d); return d; }
    d.fitKey = (m.target ? m.target.id : '') + '|' + (d.home ? d.home.lat + ',' + d.home.lon : '-');
    d.geoKey = d.fitKey + '|' + d.nodes.map(function (n) { return n.ttl + n.kind + (n.ll ? n.ll.join(',') : '') + (n.db ? '>' + n.db.join(',') : ''); }).join(';');
    d.labelKey = JSON.stringify([d.sel, m.probe, m.win, !!m.range, d.manual, d.home && d.home.city, d.geo && [d.geo.state, d.geo.month], hops.map(function (h) {
      var g = h.geo || {}, s = h.s;
      return [h.ttl, h.addr, h.status, h.origin, h.dest, s ? [nf(s.min || 0), fin(s.min), nf(s.noReplyPct || 0)] : 0, g.city, g.cc, g.country, g.verdict, g.distKm, g.vacuumMs, g.fiberMs, g.rttMs];
    })]);
    return d;
  }
  function emptyWhy(d) {
    var st = d.geo && d.geo.state;
    if (st === 'missing') return L('База мест не установлена.', 'The places database is not installed.');
    if (st === 'unreadable') return L('База мест не читается.', 'The places database cannot be read.');
    var withAddr = d.hops.filter(function (h) { return h.addr; });
    if (!withAddr.length) return L('Хопы пока не ответили.', 'No hop has answered yet.');
    if (withAddr.every(function (h) { return h.geo && h.geo.special; })) {
      return L('В маршруте только адреса вашей сети, провайдера и особых диапазонов: их мест в базе нет.', 'The route has only addresses of your network, your provider and special ranges: the database has no places for them.');
    }
    return L('В базе нет мест для адресов этого маршрута.', 'The database has no places for the addresses of this route.');
  }
  function nodeOf(ttl) { return sc ? sc.nodes.filter(function (n) { return n.ttl === ttl; })[0] || null : null; }
  function kindOf(ttl) { var n = nodeOf(ttl); return n ? n.kind : 'unknown'; }
  function groupOf(ttl) { var n = nodeOf(ttl); return n && n.g != null ? sc.groups[n.g] : null; }

  // ---------- draw ----------
  function draw() {
    if (!host || !input || !stage || !stage.isConnected) return;
    var m = input.m;
    if (!m || m.kind !== 'trace') return;
    var w = mapEl.clientWidth, h = mapEl.clientHeight, dpr = window.devicePixelRatio || 1;
    if (!w || !h) return;
    var ck = readColors(), sk = w + 'x' + h + '@' + dpr;
    if (sk !== keys.size) { resize(w, h, dpr); keys.size = sk; keys.cache = keys.l0 = keys.l2 = keys.l1 = ''; }
    if (ck !== colorKey) { colorKey = ck; keys.cache = keys.l0 = keys.l2 = keys.l1 = ''; }
    var d = derive(input);
    if (d.empty) {
      // drawn once per reason (and size or theme: the canvases were reset then)
      if (keys.empty !== d.empty || emptyEl.hidden || keys.cache !== 'empty') {
        sc = null;
        emptyEl.innerHTML = '<div><h3>' + esc(L('Мест для этого маршрута нет', 'No places for this route')) + '</h3><p>' + esc(d.empty) + '</p></div>';
        emptyEl.hidden = false;
        cx.forEach(function (c) { c.clearRect(0, 0, W, H); });
        keys.l0 = keys.l2 = keys.l1 = '';
        keys.cache = 'empty'; keys.empty = d.empty;
        hits = [];
        closeMenu();
      }
      lastD = null; an.pk = an.next = null; an.pend = 0;
      stopLoop();
    } else {
      keys.empty = null;
      emptyEl.hidden = true;
      var lk = d.geoKey + '|' + sk + '|' + furnKey(d);
      if (!sc || sc.key !== lk) layout(d, sk, lk);
      var cKey = sc.fitId + '|' + sc.step + '|' + ck + '|' + sk;
      if (keys.cache !== cKey) { drawCache(); keys.cache = cKey; keys.l0 = ''; }
      var l0 = sc.key + '|' + ck + '|' + d.sel + '|' + keys.cache;
      if (keys.l0 !== l0) { drawL0(d); keys.l0 = l0; }
      var l2 = sc.key + '|' + ck + '|' + d.labelKey;
      if (keys.l2 !== l2) { drawL2(d); keys.l2 = l2; }
      lastD = d;
      feed(d); // a reported round whose data came: its packet starts (or waits behind the one on its way)
      var wait = drawL1(d);
      if (wait >= 0) loopAfter(wait);
    }
    renderSide(d);
    if (!ready) { ready = true; window.MAP_READY = true; }
  }

  // What the layout depends on besides the places and the size: the room of the readout and of the source line.
  function furnKey(d) { return hudBox(cx[2], d).w + '|' + (d.geo ? d.geo.state + d.geo.month : ''); }
  // Projection and screen places. The bottom margin keeps the route clear of the legend; the edge pointer stops
  // short of the readout, the legend and the scale.
  function layout(d, sk, key) {
    var c = cx[2], lg = legendLayout(c, d.m), box = { w: W, h: H, top: 52, bottom: lg.h + 34 + 8 };
    if (!fitPrev || fitPrev.key !== d.fitKey + '|' + sk) choicePrev = {};
    fitPrev = G.fit(d.nodes, box, d.fitKey + '|' + sk, fitPrev);
    var p = fitPrev.proj, gs = G.gridStep(p, W, H), hud = hudBox(c, d);
    var s = G.place(d.nodes, p, W, H, [hud, lg.box, scaleLayout(c, d, hud, p, gs.step).box]);
    s.proj = p; s.fitId = fitPrev.id; s.step = gs.step; s.cell = gs.cell; s.key = key;
    s.ticks = {
      top: G.crossings(p, 0.5, true, W, gs.step), bottom: G.crossings(p, H - 0.5, true, W, gs.step),
      left: G.crossings(p, 0.5, false, H, gs.step), right: G.crossings(p, W - 0.5, false, H, gs.step),
    };
    s.labelsAt = {
      top: G.crossings(p, 11, true, W, gs.step), bottom: G.crossings(p, H - 11, true, W, gs.step),
      left: G.crossings(p, 12, false, H, gs.step), right: G.crossings(p, W - 12, false, H, gs.step),
    };
    sc = s;
  }

  // Land, the grid of 10/5/2 degrees, ruler ticks on the four edges and a small cross at every grid point (the grid
  // lines almost vanish over land; the crosses keep the grid readable).
  function drawCache() {
    var c = cacheCx, p = sc.proj, path = d3.geoPath(p, c);
    c.clearRect(0, 0, W, H);
    c.fillStyle = C.layer; c.beginPath(); path(land()); c.fill();
    c.strokeStyle = C.grid; c.lineWidth = 1; c.beginPath(); path(d3.geoGraticule().step([sc.step, sc.step])()); c.stroke();
    c.fillStyle = C.muted;
    sc.ticks.top.forEach(function (t) { if (t.at > 2 && t.at < W - 2) c.fillRect(Math.round(t.at), 0, 1, 6); });
    sc.ticks.bottom.forEach(function (t) { if (t.at > 2 && t.at < W - 2) c.fillRect(Math.round(t.at), H - 6, 1, 6); });
    sc.ticks.left.forEach(function (t) { if (t.at > 2 && t.at < H - 2) c.fillRect(0, Math.round(t.at), 6, 1); });
    sc.ticks.right.forEach(function (t) { if (t.at > 2 && t.at < H - 2) c.fillRect(W - 6, Math.round(t.at), 6, 1); });
    if (sc.cell >= 40) {
      c.fillStyle = C.faint;
      for (var lon = -180; lon < 180; lon += sc.step) {
        for (var lat = -80; lat <= 80; lat += sc.step) {
          var q = p([lon, lat]);
          if (!q || !isFinite(q[0]) || q[0] < 4 || q[0] > W - 4 || q[1] < 4 || q[1] > H - 4) continue;
          var x = Math.round(q[0]), y = Math.round(q[1]);
          c.fillRect(x - 4, y, 9, 1); c.fillRect(x, y - 4, 1, 9);
        }
      }
    }
  }

  // Route (great-circle arcs, dashed where a neighbour's place is unknown), the dotted pointer of an anycast
  // destination, place markers and the reticle on the selected hop.
  function drawL0(d) {
    var c = cx[0];
    c.clearRect(0, 0, W, H);
    c.drawImage(cache, 0, 0, W, H);
    c.strokeStyle = C.line; c.lineWidth = 2; c.lineCap = 'butt'; c.lineJoin = 'round';
    sc.segs.forEach(function (s) {
      c.setLineDash(s.dashed ? [6, 5] : []);
      s.parts.forEach(function (q) {
        c.beginPath();
        q.forEach(function (p, j) { if (j) c.lineTo(p[0], p[1]); else c.moveTo(p[0], p[1]); });
        c.stroke();
      });
    });
    c.setLineDash([]);
    if (sc.edge) {
      var e = sc.edge, from = e.line[0], end = e.inside ? e.dbp : e.point, started = false;
      c.strokeStyle = C.muted; c.lineWidth = 1.5; c.setLineDash([1.5, 4]); c.lineCap = 'round';
      c.beginPath();
      for (var i = 1; i < e.line.length; i++) {
        var p = e.line[i];
        if (G.dist(p, from) < 18) continue;
        if (G.dist(p, end) < 12) break;
        if (!started) { c.moveTo(p[0], p[1]); started = true; } else c.lineTo(p[0], p[1]);
      }
      c.stroke(); c.setLineDash([]); c.lineCap = 'butt';
    }
    // The reticle goes under the markers: a neighbour's marker covers a bracket instead of being cut by it.
    var gs = groupOf(d.sel);
    if (gs) reticle(c, gs.p[0], gs.p[1], gs.mk);
    sc.groups.forEach(function (g) { marker(c, g.mk, g.p[0], g.p[1]); });
    if (sc.edge && sc.edge.inside) mImpossible(c, sc.edge.dbp[0], sc.edge.dbp[1]);
  }

  // ---------- callouts: TTL badges with status strips and text, joined to the place by a leader ----------
  function rttPart(h) { return h.s && fin(h.s.min) ? { t: msT(h.s.min), f: 'mono', c: C.text } : null; }
  // The place label of a hop; hops in a row with the same label share one callout row.
  function labelOf(h, d) {
    var k = kindOf(h.ttl), g = h.geo || {};
    if (k === 'own') return { key: 'own', parts: [{ t: L('ваша сеть', 'your network') }].concat(d.manual ? [{ t: L('(дом задан вручную)', '(home set by hand)'), c: C.text }] : []) };
    if (k === 'city') return { key: 'c:' + g.city + g.cc + (h.dest ? ':d' : ''), main: true, rtt: true, parts: [{ t: placeName(g) }].concat(h.dest ? [{ t: L('цель', 'destination'), c: C.text }] : []) };
    if (k === 'country') return { key: 'k:' + g.cc, main: true, rtt: true, parts: [{ t: placeName(g) }] };
    if (k === 'anchored') return { key: 'a:' + h.ttl, rtt: true, parts: [{ t: h.addr, f: 'mono' }, { t: L('цель, вероятно anycast', 'destination, likely anycast'), c: C.text }] };
    if (k === 'impossible') return { key: 'i:' + h.ttl, main: true, rtt: true, parts: [{ t: L('место неизвестно', 'place unknown') }] };
    if (!h.addr) return { key: 's:' + h.status, parts: [{ t: stWord(h.status, d.m), c: C.text }] };
    return { key: 'n', main: true, rtt: true, parts: [{ t: L('место неизвестно', 'place unknown') }] };
  }
  function badgeOf(h, d) { return { ttl: h.ttl, status: h.status, sel: h.ttl === d.sel }; }
  function notesOf(run, d) {
    var out = [], multi = run.length > 1;
    run.forEach(function (h, i) {
      var k = kindOf(h.ttl), g = h.geo || {}, pre = multi ? { t: h.ttl + ':', f: 'monoS', gap: 5 } : null;
      function lead(col) { return pre ? [{ t: pre.t, f: 'monoS', c: col, gap: 5 }] : []; }
      if (h.origin) out.push({ parts: lead(C['loss-text']).concat([{ t: stOf(h, d.m), c: C['loss-text'] }]) });
      if (h.status === 'partial') {
        out.push({ parts: lead(C['partial-text']).concat([{ t: stWord('partial', d.m), c: C['partial-text'] }]) });
        if (h.s) out.push({ indent: pre, parts: [{ t: pctT(h.s.noReplyPct), f: 'mono', c: C.text }, { t: L('проб без ответа', 'of probes unanswered'), c: C.text }] });
      }
      if (h.status === 'unreachable') out.push({ parts: lead(C.text).concat([{ t: stWord('unreachable', d.m), c: C.text }]) });
      // (the picture for publication leaves out the distances from home: they would point at it)
      if (k === 'impossible' && fin(g.distKm)) {
        out.push({ parts: [{ t: L('база: ', 'database: ') + placeName(g) + (d.mask ? L(': невозможно', ': impossible') : L(': невозможно,', ': impossible,')), c: C.text }] });
        if (!d.mask) out.push({ parts: [{ t: kmT(g.distKm) + L(' требуют от ', ' need at least ') + msT(g.vacuumMs), c: C.text }] });
      }
      if (k === 'city' && g.verdict === 'doubtful' && fin(g.distKm)) {
        out.push({ parts: [{ t: L('база: ', 'database: ') + placeName(g) + (d.mask ? L(': сомнительно', ': doubtful') : L(': сомнительно,', ': doubtful,')), c: C.text }] });
        if (!d.mask) out.push({ parts: [{ t: kmT(g.distKm) + L(' по волокну требуют от ', ' in fibre need at least ') + msT(g.fiberMs), c: C.text }] });
      }
      if (k === 'country' && i === 0) out.push({ parts: [{ t: L('известна только страна', 'only the country is known'), c: C.text }] });
      if (k === 'anchored') out.push({ parts: [{ t: L('показана у последнего точного хопа', 'shown at the last hop with a known place'), c: C.text }] });
    });
    return out;
  }
  function rowsOf(hs, d) {
    var runs = [], rows = [];
    hs.forEach(function (h) {
      var lb = labelOf(h, d), last = runs[runs.length - 1];
      if (last && last.lb.key === lb.key) last.hops.push(h); else runs.push({ lb: lb, hops: [h] });
    });
    runs.forEach(function (r) {
      var sel = r.hops.some(function (h) { return h.ttl === d.sel; });
      var parts = r.lb.parts.map(function (p, i) { return i === 0 && r.lb.main && sel ? { t: p.t, f: 'sysB', c: p.c } : p; });
      var rt = r.lb.rtt ? rttPart(r.hops[0]) : null;
      if (rt) parts.push(rt);
      rows.push({ badges: r.hops.map(function (h) { return badgeOf(h, d); }), parts: parts });
      rows = rows.concat(notesOf(r.hops, d));
    });
    return rows;
  }
  function measure(c, rows, kind) {
    var pad = 6, gapB = 3, gapT = 7, textX = 0, w = 0, hh = 0;
    rows.forEach(function (r) {
      if (r.badges) textX = Math.max(textX, r.badges.reduce(function (s, b) { return s + badgeW(c, b.ttl) + gapB; }, 0) - gapB + gapT);
    });
    rows.forEach(function (r) {
      r.h = r.badges ? 24 : 17;
      r.ind = r.indent ? partW(c, r.indent) + r.indent.gap : 0;
      var pw = r.parts.reduce(function (s, p) { return s + partW(c, p) + (p.gap != null ? p.gap : 6); }, 0) - (r.parts.length ? 6 : 0);
      r.w = r.parts.length ? textX + r.ind + pw : textX - gapT;
      w = Math.max(w, r.w); hh += r.h;
    });
    hh -= 3;
    return { rows: rows, textX: textX, w: Math.ceil(w + pad * 2), h: Math.ceil(hh + pad * 2), kind: kind };
  }
  function drawPlate(c, pl, x0, y0) {
    c.fillStyle = C.well; c.fillRect(x0, y0, pl.w, pl.h);
    var y = y0 + 6;
    pl.rows.forEach(function (r) {
      var x = x0 + 6;
      if (r.badges) {
        r.badges.forEach(function (b) {
          var bw = badge(c, x, y, b);
          hits.push({ x: x, y: y, w: bw, h: 22, ttl: b.ttl, k: 0 });
          x += bw + 3;
        });
      }
      x = x0 + 6 + pl.textX + r.ind;
      var cy = r.badges ? y + 8.5 : y + 7.5;
      r.parts.forEach(function (p) {
        partFont(c, p); c.fillStyle = p.c || C.title; c.textBaseline = 'middle'; c.fillText(p.t, x, cy);
        x += c.measureText(p.t).width + (p.gap != null ? p.gap : 6);
      });
      y += r.h;
    });
  }
  // Callouts in order: selected, loss origin, destination (its edge pointer just before it), home, impossible /
  // doubtful / unreachable, partial, the rest by TTL.
  function jobsOf(d) {
    var out = [];
    sc.groups.forEach(function (g) {
      var hs = g.ttls.filter(function (t) { return t > 0; }).map(function (t) { return d.byTtl[t]; }).filter(Boolean);
      if (!hs.length) return;
      var pr = 6, sel = false;
      hs.forEach(function (h) {
        var gg = h.geo || {};
        if (h.ttl === d.sel) { pr = 0; sel = true; }
        if (h.origin) pr = Math.min(pr, 1);
        if (h.dest) pr = Math.min(pr, 2);
        if (h.status === 'unreachable' || gg.verdict === 'impossible' || gg.verdict === 'doubtful') pr = Math.min(pr, 4);
        if (h.status === 'partial') pr = Math.min(pr, 5);
      });
      if (g.mk === 'home' || g.mk === 'own') pr = Math.min(pr, 3);
      // r0: where the leader starts; rm: how far the marker reaches (the reticle around the selected one)
      out.push({ key: 'g' + hs[0].ttl, g: g, p: g.p, r0: sel ? retS(g.mk) + 3 : R0[g.mk] || 9, rm: sel ? retS(g.mk) : MR[g.mk] || 6, tan: g.tan,
        rows: rowsOf(hs, d), prio: pr, first: hs[0].ttl, badges: hs.map(function (h) { return badgeOf(h, d); }) });
    });
    // The callout at the edge pointer says where the database puts the destination. It has the least room (at the
    // map's edge), so it goes right before the destination's own callout.
    if (sc.edge && !sc.edge.inside && d.byTtl[sc.edge.ttl]) {
      var t = d.byTtl[sc.edge.ttl], g2 = t.geo || {}, own = out.filter(function (j) { return j.g.ttls.indexOf(t.ttl) >= 0; })[0];
      out.push({ key: 'edge', g: null, p: sc.edge.point, r0: 10, rm: 8, tan: [Math.cos(sc.edge.angle), Math.sin(sc.edge.angle)], prio: (own ? own.prio : 2) - 0.1, first: t.ttl,
        badges: [badgeOf(t, d)],
        rows: [
          { badges: [badgeOf(t, d)], parts: [{ t: L('по базе ', 'database says ') + placeName(g2) + L(': невозможно', ': impossible') }] },
          { parts: [{ t: d.mask ? L('ответ за ', 'reply in ') + msT(g2.rttMs) : kmT(g2.distKm) + L(', свету нужно от ', ': light needs at least ') + msT(g2.vacuumMs) + L(', ответ за ', ', reply in ') + msT(g2.rttMs), c: C.text }] },
        ] });
    }
    out.sort(function (a, b) { return a.prio - b.prio || a.first - b.first; });
    return out;
  }
  // 8 directions x leaders of 16, 40, 72 px; N and S also shifted by a third of the plate: 36 places to try.
  var DIRS = [['E', 1, 0], ['NE', 1, -1], ['N', 0, -1], ['NW', -1, -1], ['W', -1, 0], ['SW', -1, 1], ['S', 0, 1], ['SE', 1, 1]];
  function candidates(job, pl) {
    var out = [];
    DIRS.forEach(function (dd) {
      var n = Math.sqrt(dd[1] * dd[1] + dd[2] * dd[2]), ux = dd[1] / n, uy = dd[2] / n;
      [16, 40, 72].forEach(function (len) {
        var R = job.r0 + len, ax = job.p[0] + ux * R, ay = job.p[1] + uy * R;
        (dd[1] === 0 ? [0, -pl.w / 3, pl.w / 3] : [0]).forEach(function (sh) {
          var x = dd[1] > 0 ? ax : dd[1] < 0 ? ax - pl.w : ax - pl.w / 2 + sh;
          var y = dd[2] > 0 ? ay : dd[2] < 0 ? ay - pl.h : ay - pl.h / 2;
          out.push({ id: dd[0] + len + (sh > 0 ? '+' : sh < 0 ? '-' : ''), x: Math.round(x), y: Math.round(y), ux: ux, uy: uy });
        });
      });
    });
    return out;
  }
  // Leader from the marker edge (radius r0) to the nearest point of the plate; none when the plate is right there.
  function leaderOf(job, b) {
    var p = job.p, nx = Math.min(Math.max(p[0], b.x), b.x + b.w), ny = Math.min(Math.max(p[1], b.y), b.y + b.h);
    var dd = Math.sqrt((nx - p[0]) * (nx - p[0]) + (ny - p[1]) * (ny - p[1]));
    if (dd <= job.r0 + 2) return null;
    return [p[0] + (nx - p[0]) * job.r0 / dd, p[1] + (ny - p[1]) * job.r0 / dd, nx, ny];
  }
  function ccw(ax, ay, bx, by, cxp, cyp) { return (cyp - ay) * (bx - ax) > (by - ay) * (cxp - ax); }
  function segX(a, b) {
    return ccw(a[0], a[1], b[0], b[1], b[2], b[3]) !== ccw(a[2], a[3], b[0], b[1], b[2], b[3]) &&
      ccw(a[0], a[1], a[2], a[3], b[0], b[1]) !== ccw(a[0], a[1], a[2], a[3], b[2], b[3]);
  }
  function inBox(x, y, b) { return x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h; }
  function segBox(s, b) {
    if (inBox(s[0], s[1], b) || inBox(s[2], s[3], b)) return true;
    var x0 = b.x, y0 = b.y, x1 = b.x + b.w, y1 = b.y + b.h;
    return segX(s, [x0, y0, x1, y0]) || segX(s, [x1, y0, x1, y1]) || segX(s, [x1, y1, x0, y1]) || segX(s, [x0, y1, x0, y0]);
  }
  function boxDist(b, p) {
    var nx = Math.min(Math.max(p[0], b.x), b.x + b.w), ny = Math.min(Math.max(p[1], b.y), b.y + b.h);
    return Math.sqrt((nx - p[0]) * (nx - p[0]) + (ny - p[1]) * (ny - p[1]));
  }
  // A plate reads as the label of the marker it hugs: another hop's marker may come no closer to it than this (px,
  // edge to edge), and never closer than the plate's own marker.
  var CLEAR = 14;
  // Not allowed: leaving the map's 8 px inset; covering a plate, a marker, a leader or the furniture (readout, legend,
  // scale, edge arrow); a leader through a plate, another hop's marker or the furniture; a plate that another hop's
  // marker hugs (it would read as that hop's label). Returns the plate's box and leader, or null.
  function valid(job, pl, cd, ob) {
    var b = { x: cd.x, y: cd.y, w: pl.w, h: pl.h }, i, mk;
    if (b.x < 8 || b.y < 8 || b.x + b.w > W - 8 || b.y + b.h > H - 8) return null;
    for (i = 0; i < ob.taken.length; i++) if (hitB(b, ob.taken[i])) return null;
    for (i = 0; i < ob.plates.length; i++) if (hitB(b, ob.plates[i])) return null;
    for (i = 0; i < ob.marks.length; i++) if (hitB(b, ob.marks[i])) return null;
    for (i = 0; i < ob.leaders.length; i++) if (segBox(ob.leaders[i], b)) return null;
    var ld = leaderOf(job, b), near = Math.min(CLEAR, boxDist(b, job.p) - job.rm + 4);
    if (ld) {
      for (i = 0; i < ob.plates.length; i++) if (segBox(ld, ob.plates[i])) return null;
      for (i = 0; i < ob.marks.length; i++) if (ob.marks[i].g !== job.g && segBox(ld, ob.marks[i].lb)) return null;
      for (i = 0; i < ob.taken.length; i++) if (ob.taken[i].own !== job.key && segBox(ld, ob.taken[i])) return null;
    }
    for (i = 0; i < ob.marks.length; i++) {
      mk = ob.marks[i];
      if (mk.g !== job.g && boxDist(b, mk.p) - mk.r < near) return null;
    }
    return { b: b, ld: ld };
  }
  // Cost: 3 per covered route sample, 25 per leader crossing (route or another leader), 0.2 per px of leader,
  // 10 x |cos| of the direction against the route there, 15 for leaving the place of the last layout.
  function score(job, pl, cd, ob) {
    var v = valid(job, pl, cd, ob), i;
    if (!v) return null;
    var b = v.b, ld = v.ld, cov = cover(b), cost = 3 * cov;
    if (ld) {
      var n = 0;
      for (i = 0; i < ob.lines.length; i++) if (segX(ld, ob.lines[i])) n++;
      for (i = 0; i < ob.leaders.length; i++) if (segX(ld, ob.leaders[i])) n++;
      cost += 25 * n + 0.2 * Math.sqrt((ld[2] - ld[0]) * (ld[2] - ld[0]) + (ld[3] - ld[1]) * (ld[3] - ld[1]));
    }
    if (job.tan) cost += 10 * Math.abs(cd.ux * job.tan[0] + cd.uy * job.tan[1]);
    var prev = choicePrev[job.key];
    if (prev && prev !== pl.kind + cd.id) cost += 15;
    return { cost: cost, b: b, ld: ld, id: pl.kind + cd.id, cov: cov };
  }
  // Route samples under a box.
  function cover(b) {
    var n = 0;
    for (var i = 0; i < sc.samples.length; i++) if (inBox(sc.samples[i][0], sc.samples[i][1], b)) n++;
    return n;
  }
  // In order, each callout takes the best place for its full plate, else for its compact one (badges only); a place
  // that would leave a later callout no room even for its compact plate is skipped while another one does. A plate
  // should not hide a stretch of the route (more than ROUTE samples): the line breaks under it, in the picture too.
  // When one does, the pass runs again with a wider look ahead: of the places above, a full plate takes the
  // cheapest that leaves the fewest plates on the route (itself, and each later callout whose last place off the
  // route it would take); that layout is kept when it hides less of the route and labels as much. When a hop still
  // goes without any plate (a small map), two passes instead: first every callout gets its compact plate, in order,
  // so no hop loses its number to a big plate placed before it; then, in the same order, a compact plate grows into
  // the full one wherever the full one fits around all the others.
  var ROUTE = 2;
  function placeCallouts(c, d, ob) {
    var jobs = jobsOf(d), next = {}, at;
    jobs.forEach(function (job) { job.pf = measure(c, job.rows, 'f'); job.pc = measure(c, [{ badges: job.badges, parts: [] }], 'c'); });
    function others(list, i) {
      var o = { taken: ob.taken, marks: ob.marks, lines: ob.lines, plates: [], leaders: [] };
      list.forEach(function (a, j) { if (a && j !== i) { o.plates.push(a.b); if (a.ld) o.leaders.push(a.ld); } });
      return o;
    }
    function fits(job, o) { return candidates(job, job.pc).some(function (cd) { return !!valid(job, job.pc, cd, o); }); }
    // Room for the full plate off the route (which of its places are off the route, its box alone decides).
    function fitsOff(job, o) {
      if (!job.offC) job.offC = candidates(job, job.pf).filter(function (cd) { return cover({ x: cd.x, y: cd.y, w: job.pf.w, h: job.pf.h }) <= ROUTE; });
      return job.offC.some(function (cd) { return !!valid(job, job.pf, cd, o); });
    }
    function plus(o, r) { return { taken: o.taken, marks: o.marks, lines: o.lines, plates: o.plates.concat([r.b]), leaders: r.ld ? o.leaders.concat([r.ld]) : o.leaders }; }
    // Room for the compact plate of every callout in rest once plate r is placed.
    function roomFor(rest, o, r) {
      var o2 = plus(o, r);
      return rest.every(function (j) { return fits(j, o2); });
    }
    // Plates on the route once plate r is placed: r itself, and each callout in off left without a place off it.
    function onRoute(off, o, r) {
      var o2 = plus(o, r);
      return (r.cov > ROUTE ? 1 : 0) + off.filter(function (j) { return !fitsOff(j, o2); }).length;
    }
    // The cheapest place for the plate; given the callouts still to come (rest), the cheapest that leaves each of
    // them room for its compact plate; given off (the later callouts that still have a place off the route for
    // their full plate), the cheapest of those that leave the fewest plates on the route.
    function best(job, pl, o, rest, off) {
      var list = [], pick = null;
      candidates(job, pl).forEach(function (cd, ci) { var r = score(job, pl, cd, o); if (r) { r.ci = ci; list.push(r); } });
      list.sort(function (a, b) { return Math.abs(a.cost - b.cost) > 1e-9 ? a.cost - b.cost : a.ci - b.ci; });
      for (var k = 0; k < list.length && !(pick && pick.bad === 0); k++) {
        var r = list[k];
        if (rest && !roomFor(rest, o, r)) continue;
        r.bad = off ? onRoute(off, o, r) : 0;
        if (!pick || r.bad < pick.bad) pick = r;
      }
      if (pick) pick.pl = pl;
      return pick;
    }
    function missing(list) { return jobs.filter(function (j, i) { return !list[i]; }).length; }
    // One pass in order (look: with the wider look ahead).
    function pass(look) {
      var list = [];
      jobs.forEach(function (job, i) {
        var o = others(list, i), later = jobs.slice(i + 1), rest = later.filter(function (j) { return fits(j, o); });
        var off = look ? later.filter(function (j) { return fitsOff(j, o); }) : null;
        list[i] = best(job, job.pf, o, rest, off) || best(job, job.pc, o, rest) || best(job, job.pf, o) || best(job, job.pc, o);
      });
      return list;
    }
    // Plates on the route and the route samples under them; full plates.
    function hid(list) { var n = 0, s = 0; list.forEach(function (a) { if (a && a.cov > ROUTE) { n++; s += a.cov; } }); return [n, s]; }
    function fulls(list) { return list.filter(function (a) { return a && a.pl.kind === 'f'; }).length; }
    var one = pass(false), h1 = hid(one);
    if (h1[0]) {
      var alt = pass(true), h2 = hid(alt);
      if (missing(alt) <= missing(one) && fulls(alt) >= fulls(one) && (h2[0] < h1[0] || (h2[0] === h1[0] && h2[1] < h1[1]))) one = alt;
    }
    at = one;
    if (missing(one)) {
      var two = [];
      jobs.forEach(function (job, i) {
        var o = others(two, i), rest = jobs.slice(i + 1).filter(function (j) { return fits(j, o); });
        two[i] = best(job, job.pc, o, rest) || best(job, job.pc, o);
      });
      jobs.forEach(function (job, i) {
        var full = two[i] ? best(job, job.pf, others(two, i)) : null;
        if (full) two[i] = full;
      });
      if (missing(two) < missing(one)) at = two;
    }
    c.strokeStyle = C.muted; c.lineWidth = 1;
    at.forEach(function (a) {
      if (!a || !a.ld) return;
      c.beginPath(); c.moveTo(a.ld[0], a.ld[1]); c.lineTo(a.ld[2], a.ld[3]); c.stroke();
      ob.leaders.push(a.ld);
    });
    jobs.forEach(function (job, i) {
      var a = at[i];
      if (!a) return;
      drawPlate(c, a.pl, a.b.x, a.b.y);
      hits.push({ x: a.b.x, y: a.b.y, w: a.b.w, h: a.b.h, ttl: job.first, k: 1 });
      ob.plates.push(a.b);
      next[job.key] = a.id;
    });
    choicePrev = next;
  }

  // ---------- legend, scale and sources, degree labels ----------
  function legendLayout(c, m) {
    var pad = 8, rows = [
      { k: L('Место', 'Place'), items: [['home', L('ваша сеть', 'your network')], ['city', L('город', 'city')], ['country', L('страна', 'country')],
        ['unknown', L('неизвестно', 'unknown')], ['impossible', L('невозможно', 'impossible')]] },
      { k: L('Статус', 'Status'), items: [['loss', stWord('loss', m)], ['partial', stWord('partial', m)], ['silent', stWord('silent', m)]] },
      { k: L('Линия', 'Line'), items: [['solid', L('места известны', 'places known')], ['dashed', L('место соседа неизвестно', "neighbour's place unknown")]] },
      { k: '', items: [['ttl', L('номер хопа, как в таблице', 'hop number, as in the table')]] },
    ];
    font(c, 12, SYS);
    var kW = Math.max.apply(null, rows.map(function (r) { return c.measureText(r.k).width; })) + 12, maxW = W - 16 - pad * 2 - kW, lines = [];
    rows.forEach(function (r) {
      var cur = { k: r.k, items: [], w: 0 };
      r.items.forEach(function (it) {
        var iw = (it[0] === 'ttl' ? 44 : 21) + c.measureText(it[1]).width;
        if (cur.items.length && cur.w + 14 + iw > maxW) { lines.push(cur); cur = { k: '', items: [], w: 0 }; }
        cur.w += (cur.items.length ? 14 : 0) + iw;
        cur.items.push({ g: it[0], label: it[1], w: iw });
      });
      lines.push(cur);
    });
    // the picture has the credit in its sources line
    var credit = XP ? '' : 'IP Geolocation by DB-IP (db-ip.com), CC BY 4.0';
    font(c, 11, SYS);
    var crw = c.measureText(credit).width;
    var w = Math.ceil(pad * 2 + Math.max(kW + Math.max.apply(null, lines.map(function (l) { return l.w; })), crw)), h = pad * 2 + lines.length * 20 + (credit ? 12 : -4);
    return { lines: lines, kW: kW, w: w, h: h, credit: credit, box: { x: 8, y: H - 34 - h, w: w, h: h } };
  }
  function drawLegend(c, lg) {
    var b = lg.box, pad = 8;
    c.fillStyle = C.well; c.fillRect(b.x, b.y, b.w, b.h);
    lg.lines.forEach(function (ln, i) {
      var cy = b.y + pad + i * 20 + 8, x = b.x + pad + lg.kW;
      font(c, 12, SYS); c.fillStyle = C.muted; c.textBaseline = 'middle';
      if (ln.k) c.fillText(ln.k, b.x + pad, cy);
      ln.items.forEach(function (it) {
        var gx = x + 7, tx = x + 21;
        if (it.g === 'home') mHome(c, gx, cy + 0.5);
        else if (it.g === 'city') mCity(c, gx, cy);
        else if (it.g === 'country') mCountry(c, gx, cy, 6.5);
        else if (it.g === 'unknown') mUnknown(c, gx, cy, 7);
        else if (it.g === 'impossible') mImpossible(c, gx, cy, 14);
        else if (it.g === 'solid' || it.g === 'dashed') {
          c.strokeStyle = C.line; c.lineWidth = 2; c.setLineDash(it.g === 'dashed' ? [4, 3] : []);
          c.beginPath(); c.moveTo(x - 1, cy); c.lineTo(x + 15, cy); c.stroke(); c.setLineDash([]);
        } else if (it.g === 'ttl') {
          var w1 = badge(c, x - 1, cy - 8, { ttl: 1, status: 'ok' });
          badge(c, x + w1 + 2, cy - 8, { ttl: 2, status: 'ok' });
          tx = x + 44;
        } else strip(c, x - 1, cy - 2, 17, it.g);
        font(c, 12, SYS); c.fillStyle = C.text; c.textBaseline = 'middle';
        c.fillText(it.label, tx, cy);
        x += it.w + 14;
      });
    });
    if (!lg.credit) return;
    font(c, 11, SYS); c.fillStyle = C.muted; c.textBaseline = 'middle';
    c.fillText(lg.credit, b.x + pad, b.y + b.h - pad - 5);
  }
  // Scale bar at the centre latitude and the source line under it (top right; under the readout when they meet).
  function scaleLayout(c, d, hud, proj, step) {
    var mid = G.inv(proj, [W / 2, H / 2]) || [0, 0], lat = Math.round(mid[1]);
    var dLon = 100 / (111.32 * Math.max(0.05, Math.cos(lat * Math.PI / 180)));
    var a = proj([mid[0], lat]), b = proj([mid[0] + dLon, lat]), px100 = a && b ? G.dist(a, b) : 50, km = 10;
    [10, 20, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000].forEach(function (k) { if (px100 * k / 100 <= 140) km = k; });
    var len = Math.max(8, Math.round(px100 * km / 100)), short = kmT(km);
    var label = short + ' ' + L('(на ', '(at ') + Math.abs(lat) + '°' + NB + ns(lat) + ')';
    // the month of the database in use only: an unreadable set has a month too (Snapshot.geo)
    var geo = d.geo, mo = geo && geo.state === 'ok' && geo.month ? ' ' + monthShort(geo.month) : '', sim = geo && geo.state === 'simulated';
    var nat = L('суша Natural Earth', 'land: Natural Earth'), grid = L('сетка ', 'grid ') + step + '°';
    var ip = sim ? L('места: имитация', 'places: simulated') : 'IP: DB-IP' + mo;
    font(c, 11, MONO);
    var x1 = W - 14, y = 30;
    function plate(lb, lines) {
      var w = Math.max.apply(null, [c.measureText(lb).width + 8 + len].concat(lines.map(function (s) { return c.measureText(s).width; })));
      return { label: lb, lines: lines, w: Math.ceil(w) + 12, h: 26 + lines.length * 14 };
    }
    function fits(p) { return x1 - p.w + 6 >= hud.x + hud.w + 8; }
    // One source line; on a narrow map two lines and the bare scale; under the readout when even that does not fit.
    var pl = plate(label, [nat + ', ' + ip + ', ' + grid]);
    if (!fits(pl)) pl = plate(label, [nat + ', ' + grid, ip]);
    if (!fits(pl)) pl = plate(short, [nat + ', ' + grid, ip]);
    if (!fits(pl)) pl = plate(label, [nat + ', ' + grid, ip]);
    var box = { x: x1 - pl.w + 6, y: y - 22, w: pl.w, h: pl.h };
    if (!fits(pl)) { y += 36; box.y += 36; }
    return { len: len, label: pl.label, lines: pl.lines, x1: x1, y: y, box: box };
  }
  function drawScale(c, sp) {
    var b = sp.box, x1 = sp.x1, y = sp.y, len = sp.len;
    c.fillStyle = C.well; c.fillRect(b.x, b.y, b.w, b.h);
    c.fillStyle = C.title;
    c.fillRect(x1 - len, y - 10, len, 2); c.fillRect(x1 - len, y - 14, 2, 6); c.fillRect(x1 - 2, y - 14, 2, 6);
    c.fillRect(x1 - Math.round(len / 2), y - 12, 1, 4);
    font(c, 11, MONO); c.fillStyle = C.text; c.textAlign = 'right'; c.textBaseline = 'middle';
    c.fillText(sp.label, x1 - len - 8, y - 9);
    c.fillStyle = C.muted;
    sp.lines.forEach(function (s, i) { c.fillText(s, x1, y + 8 + i * 14); });
    c.textAlign = 'left';
  }
  // Degree labels on all four edges, on sea-coloured plates; a label that meets anything already drawn is skipped.
  function degreeLabels(c, taken) {
    var list = [], placed = [];
    sc.labelsAt.bottom.forEach(function (t) { if (t.at > 40 && t.at < W - 40) list.push({ s: degLon(t.v), x: t.at, y: H - 11, a: 'c' }); });
    sc.labelsAt.left.forEach(function (t) { if (t.at > 50 && t.at < H - 40) list.push({ s: degLat(t.v), x: 8, y: t.at, a: 'l' }); });
    sc.labelsAt.right.forEach(function (t) { if (t.at > 50 && t.at < H - 40) list.push({ s: degLat(t.v), x: W - 8, y: t.at, a: 'r' }); });
    sc.labelsAt.top.forEach(function (t) { if (t.at > 40 && t.at < W - 40) list.push({ s: degLon(t.v), x: t.at, y: 11, a: 'c' }); });
    font(c, 11, MONO);
    list.forEach(function (g) {
      var w = c.measureText(g.s).width, x0 = g.a === 'c' ? g.x - w / 2 : g.a === 'r' ? g.x - w : g.x;
      var b = { x: Math.round(x0 - 3), y: Math.round(g.y - 8), w: Math.round(w + 6), h: 16 };
      if (b.x < 2 || b.x + b.w > W - 2) return;
      if (taken.some(function (t) { return hitB(b, t); }) || placed.some(function (t) { return hitB(b, t); })) return;
      c.fillStyle = C.well; c.fillRect(b.x, b.y, b.w, b.h);
      c.fillStyle = C.muted; c.textBaseline = 'middle'; c.fillText(g.s, x0, g.y + 0.5);
      placed.push(b);
    });
  }
  function drawL2(d) {
    var c = cx[2];
    c.clearRect(0, 0, W, H);
    hits = [];
    var hud = hudBox(c, d), lg = legendLayout(c, d.m), sp = scaleLayout(c, d, hud, sc.proj, sc.step);
    var ob = { taken: [hud, lg.box, sp.box], plates: [], leaders: [], lines: [], marks: [] };
    if (sc.edge && !sc.edge.inside) ob.taken.push({ x: sc.edge.point[0] - 10, y: sc.edge.point[1] - 10, w: 20, h: 20, own: 'edge' });
    // Markers: the box a plate may not cover, the group they belong to, the point and how far the marker reaches.
    var selG = groupOf(d.sel);
    sc.groups.forEach(function (g) {
      var r = g === selG ? retS(g.mk) : MR[g.mk] || 6, e = g === selG ? r + 3 : r + 2;
      ob.marks.push({ x: g.p[0] - e, y: g.p[1] - e, w: 2 * e, h: 2 * e, g: g, p: g.p, r: r, lb: { x: g.p[0] - r - 4, y: g.p[1] - r - 4, w: 2 * r + 8, h: 2 * r + 8 } });
      if (g.ttls.some(function (t) { return t > 0; })) hits.push({ x: g.p[0] - 10, y: g.p[1] - 10, w: 20, h: 20, g: g, k: 2 });
    });
    if (sc.edge && sc.edge.inside) {
      var q = sc.edge.dbp;
      ob.marks.push({ x: q[0] - 10, y: q[1] - 10, w: 20, h: 20, g: 'db', p: q, r: 8, lb: { x: q[0] - 12, y: q[1] - 12, w: 24, h: 24 } });
    }
    sc.segs.forEach(function (s) {
      s.parts.forEach(function (q) { for (var i = 1; i < q.length; i++) ob.lines.push([q[i - 1][0], q[i - 1][1], q[i][0], q[i][1]]); });
    });
    placeCallouts(c, d, ob);
    if (sc.edge && !sc.edge.inside) {
      var ex = sc.edge.point[0], ey = sc.edge.point[1], a = sc.edge.angle;
      c.fillStyle = C.title; c.beginPath();
      c.moveTo(ex + Math.cos(a) * 8, ey + Math.sin(a) * 8);
      c.lineTo(ex + Math.cos(a + 2.4) * 7, ey + Math.sin(a + 2.4) * 7);
      c.lineTo(ex + Math.cos(a - 2.4) * 7, ey + Math.sin(a - 2.4) * 7);
      c.closePath(); c.fill();
    }
    drawLegend(c, lg);
    drawScale(c, sp);
    degreeLabels(c, ob.taken.concat(ob.plates, ob.marks));
  }

  // ---------- L1: the packet and its readout ----------
  function hudLine(c, parts) {
    var pad = 8, w = parts.reduce(function (s, p) { return s + partW(c, p) + (p.gap != null ? p.gap : 10); }, -10), x = 8 + pad;
    c.fillStyle = C.well; c.fillRect(8, 8, Math.ceil(w + pad * 2), 28);
    parts.forEach(function (p) {
      partFont(c, p); c.fillStyle = p.c || C.title; c.textBaseline = 'middle'; c.fillText(p.t, x, 22.5);
      x += c.measureText(p.t).width + (p.gap != null ? p.gap : 10);
    });
  }
  // Why no packet moves (the whole route is shown): paused with the button, or because the system asks for less
  // motion; a picked period; the target on pause. null: the packet follows the rounds.
  var WHY = {
    paused: L('пауза, маршрут показан целиком', 'paused, whole route shown'),
    calm: L('анимация выключена в системе', 'animation is off in the system'),
    range: L('выделен период, маршрут показан целиком', 'period selected, whole route shown'),
    target: L('цель на паузе, маршрут показан целиком', 'target paused, whole route shown'),
    whole: L('маршрут показан целиком', 'whole route shown'),
    wait: L('ждет следующий раунд', 'waiting for the next round'),
  };
  function staticWhy(d) {
    if (paused) return calm ? WHY.calm : WHY.paused;
    if (d.m.range) return WHY.range;
    if (d.m.paused) return WHY.target;
    return null;
  }
  // The readout's room on the map: the widest of its lines (every reason, moving, lost, arrived), so callouts never
  // go under it. The picture has no readout.
  function hudBox(c, d) {
    if (XP) return { x: 8, y: 8, w: 0, h: 0 };
    var rn = { t: L('раунд ', 'round ') + Math.max(99999, an.n, d.m.rounds || 0), c: C.muted }, mx = d.hops.length ? d.hops[d.hops.length - 1].ttl : 1, w = 0;
    Object.keys(WHY).map(function (k) { return [{ t: L('пакет', 'packet') }, { t: WHY[k] }]; }).concat([
      [rn, { t: L('хоп ', 'hop ') + mx + ' → ' + mx, f: 'mono' }, { t: 'RTT ' + ms1(999.9), f: 'mono' }],
      [rn, { t: L('потеря на хопе ', 'lost at hop ') + mx }, { t: L('ответа нет', 'no reply') }],
      [rn, { t: L('ответ цели', 'target replied') }, { t: ms1(999.9), f: 'mono' }],
    ]).forEach(function (ps) { w = Math.max(w, ps.reduce(function (s, p) { return s + partW(c, p) + 10; }, -10) + 16); });
    return { x: 8, y: 8, w: Math.ceil(w), h: 28 };
  }
  // Where a round of the current data ends: at the destination, or, when the destination's last round was lost, at
  // the start of the trailing run of hops whose last round was lost (silent hops skipped), as the origin rule.
  function lossIndex(d, pth) {
    var hs = d.hops, dest = hs.filter(function (h) { return h.dest; })[0] || hs[hs.length - 1], at = -1;
    if (!dest || !dest.full || !dest.full.lastLost) return -1;
    for (var i = hs.length - 1; i >= 0; i--) {
      var h = hs[i];
      if (h.status === 'silent' || !h.full) continue;
      if (h.full.lastLost) at = h.ttl; else break;
    }
    for (var k = 0; k < pth.length; k++) if (pth[k].ttl === at) return k;
    return -1;
  }
  function lossTtl(d, pth) { var i = lossIndex(d, pth); return i >= 0 ? pth[i].ttl : null; }
  function diamond(c, x, y, r) { c.beginPath(); c.moveTo(x, y - r); c.lineTo(x + r, y); c.lineTo(x, y + r); c.lineTo(x - r, y); c.closePath(); c.fill(); }

  // ---------- the packet of one round ----------
  // The clock: every hop at its window minimum, hops without one in between, made monotone (MAPG.clock); the whole
  // route takes 2.4 s (0.008-0.3 s per ms of RTT); a trail of five squares 0.017 c_dest apart. A lost round stops at
  // the hop where its loss starts and leaves a red cross there for 1.5 s; a reply rests at the target for 0.7 s (as
  // in the approved mockup).
  var REST = 0.7, CROSS = 1.5, FRAME = 33, rid = 0;
  // The packet's clock with this draw's numbers: the layout keeps the path, the window minimums move with live data.
  function retime(d) {
    G.clock(sc.path, function (ttl) { var h = d.byTtl[ttl]; return h && h.full ? h.full.min : null; });
  }
  // A round of the current data. lostAt: the ttl where it is lost (null: it reached the target; undefined: as the
  // data says). It keeps its own clock, so a snapshot that comes while it is on its way does not move it.
  function roundOf(d, lostAt) {
    retime(d);
    var pth = sc.path, n = pth.length, c = {}, li = lostAt !== undefined ? lostAt : lossTtl(d, pth), stop = null;
    pth.forEach(function (e) { c[e.ttl] = e.c; if (e.ttl === li) stop = e.c; });
    if (stop == null) li = null;
    var cd = n ? pth[n - 1].c : 1, spm = Math.max(0.008, Math.min(0.3, 2.4 / cd)), last = n ? d.byTtl[pth[n - 1].ttl] : null;
    // where a reply ends (the target, or the last hop that answers) and its time: the latest reply (this round's),
    // else the minimum
    var ms = last && last.full && fin(last.full.last) ? last.full.last : n && pth[n - 1].raw != null ? pth[n - 1].raw : null;
    if (stop == null) stop = cd;
    return {
      id: ++rid, c: c, li: li, spm: spm, move: stop * spm, end: stop * spm + (li != null ? CROSS : REST), ms: ms, n: d.m.rounds || 1, t0: 0,
      dest: !last || !!last.dest, endTtl: n ? pth[n - 1].ttl : null, sig: pth.map(function (e) { return e.c.toFixed(2); }).join(',') + '>' + li,
    };
  }
  // The path with the round's own clock (the points are the layout's: they follow a resize).
  function timed(R) {
    return sc.path.map(function (e) { return { ttl: e.ttl, p: e.p, poly: e.poly, raw: e.raw, c: R.c[e.ttl] != null ? R.c[e.ttl] : e.c }; });
  }
  function cross(c, p) {
    c.fillStyle = C.well; c.fillRect(p[0] - 10, p[1] - 10, 20, 20);
    c.strokeStyle = C.loss; c.lineWidth = 3.5; c.lineCap = 'square'; c.beginPath();
    c.moveTo(p[0] - 6.5, p[1] - 6.5); c.lineTo(p[0] + 6.5, p[1] + 6.5); c.moveTo(p[0] + 6.5, p[1] - 6.5); c.lineTo(p[0] - 6.5, p[1] + 6.5); c.stroke();
    c.lineCap = 'butt';
  }
  // The packet's head: a diamond on a sea-coloured knockout, so it reads over the route.
  function head(c, p) { c.fillStyle = C.well; diamond(c, p[0], p[1], 8.5); c.fillStyle = C.packet; diamond(c, p[0], p[1], 5.5); }
  function hudRound(c, R) {
    var rn = { t: L('раунд ', 'round ') + R.n, c: C.muted };
    if (R.li != null) hudLine(c, [rn, { t: L('потеря на хопе ', 'lost at hop ') + R.li, c: C['loss-text'] }, { t: L('ответа нет', 'no reply'), c: C.text }]);
    else {
      hudLine(c, [rn, { t: R.dest ? L('ответ цели', 'target replied') : L('ответ хопа ', 'reply from hop ') + R.endTtl, c: C.text }]
        .concat(R.ms != null ? [{ t: ms1(R.ms), f: 'mono', c: C.title }] : []));
    }
  }
  // Frame t seconds into round R, or null once the round is over. Returns the hop for the ribbon's cursor.
  function frame(c, d, t, R) {
    var pth = timed(R), n = pth.length, spm = R.spm, r = t / spm, li = -1, k;
    if (!n || t >= R.end) return null;
    for (k = 0; k < n; k++) if (pth[k].ttl === R.li) li = k;
    if (li >= 0 && r >= pth[li].c) { cross(c, pth[li].p); hudRound(c, R); return { ttl: R.li, lost: true }; }
    if (li < 0 && r >= pth[n - 1].c) { head(c, pth[n - 1].p); hudRound(c, R); return { ttl: pth[n - 1].ttl, lost: false }; }
    var pos = G.posAt(pth, r), cd = pth[n - 1].c;
    c.fillStyle = C.title;
    [4, 4, 3, 3, 2].forEach(function (s, i) {
      var rr = r - (i + 1) * 0.017 * cd;
      if (rr < 0) return;
      var q = G.posAt(pth, rr).p;
      c.fillRect(Math.round(q[0] - s / 2), Math.round(q[1] - s / 2), s, s);
    });
    head(c, pos.p);
    // ttl 0 is the home: «дом → 1» for the first moments of a round
    hudLine(c, [{ t: L('раунд ', 'round ') + R.n, c: C.muted }, { t: (pos.lastTtl ? L('хоп ', 'hop ') + pos.lastTtl : L('дом', 'home')) + ' → ' + (pos.nextTtl != null ? pos.nextTtl : pos.lastTtl), f: 'mono', c: C.text },
      { t: 'RTT ' + ms1(r), f: 'mono', c: C.title }]);
    return { ttl: pos.lastTtl || null, lost: false };
  }
  function demoRound(d) { var R = roundOf(d, demoLost); R.id = 'demo'; return R; }
  // L1: the packet on its way; else the last round's result (or that a round is awaited), or why the map is static.
  // Returns how long this picture holds, in ms (-1: until something changes).
  function drawL1(d) {
    var c = cx[1], why = staticWhy(d), R = null, t = 0, cur = null;
    if (hold) { if (frozen != null) { R = demoRound(d); t = frozen; } }
    else if (!why) { advance(); R = an.pk; t = R ? (now() - R.t0) / 1000 : 0; }
    var live = !!R && t < R.end, moving = live && t < R.move;
    var k = (live ? (moving ? 'm' + t : 'e' + R.id) + (hold ? R.sig : '') : 's' + (why || (hold ? '' : an.last ? 'r' + an.last.id : 'w'))) + '|' + keys.l0 + '|' + keys.l2;
    if (k !== keys.l1) {
      keys.l1 = k;
      c.clearRect(0, 0, W, H);
      cur = live ? frame(c, d, t, R) : null;
      if (!cur) hudIdle(c, why);
      cursor = cur ? { ttl: cur.ttl, lost: cur.lost } : { ttl: null, lost: false };
    }
    if (hold || !live) return -1;
    return moving ? FRAME : Math.max(1, Math.ceil((R.end - t) * 1000));
  }
  function hudIdle(c, why) {
    var pk = { t: L('пакет', 'packet'), c: C.muted };
    if (why || hold) hudLine(c, [pk, { t: why || WHY.whole, c: C.text }]);
    else if (an.last) hudRound(c, an.last);
    else hudLine(c, [pk, { t: WHY.wait, c: C.text }]);
  }

  // ---------- live rounds ----------
  // A "round" event of the traced target (MAP.round) is noted with its time; the first snapshot asked for after it
  // (Live.askedAt) has that round in it, and its numbers make the round's packet. One packet is on its way at a time:
  // a round that comes meanwhile waits for it, a newer one takes the waiting one's place. While the map is static
  // (WHY) there are no packets and rounds are not kept. an: pend (time of the oldest round not yet in the data),
  // newest, snap (WT.trace then: without Live.askedAt any newer snapshot counts), ev (rounds reported), n (number of
  // the last packet's round), pk (on its way), next (waiting), last (ended last: the readout keeps its result), raf
  // and timer (the frame loop), vis (the map is on screen), min (the window is minimised), tid (the row the rounds
  // are of).
  var an = { pend: 0, newest: 0, snap: null, ev: 0, evAt: 0, n: 0, pk: null, next: null, last: null, raf: 0, timer: 0, vis: true, min: false, minAt: 0, minPoll: 0, io: null, tid: undefined };
  function now() { return window.performance && performance.now ? performance.now() : Date.now(); }
  function onRound() {
    if (hold) return;
    var t = Date.now();
    an.ev++;
    if (!an.pend) { an.pend = t; an.snap = WT.trace; }
    an.newest = t;
  }
  function fresh() {
    var lv = window.Live;
    return lv && lv.askedAt ? lv.askedAt >= an.pend : WT.trace !== an.snap;
  }
  function quiet() { an.pk = an.next = an.last = null; an.pend = 0; stopLoop(); }
  function feed(d) {
    if (hold) return;
    // another row of the target (the probe switch): its packets and their numbers start afresh
    var tid = d.m.target ? d.m.target.id : null;
    if (tid !== an.tid) {
      if (an.tid !== undefined) { an.n = 0; an.evAt = an.ev; quiet(); }
      an.tid = tid;
    }
    if (staticWhy(d)) { quiet(); return; }
    if (!an.pend || !fresh()) return;
    var lv = window.Live, asked = lv && lv.askedAt ? lv.askedAt : Date.now(), R = roundOf(d);
    // the round's number: the first packet's is the window's rounds, then one more for every reported round (another
    // window does not make it jump)
    R.n = an.n ? an.n + Math.max(1, an.ev - an.evAt) : Math.max(1, d.m.rounds || 0);
    an.n = R.n; an.evAt = an.ev;
    an.pend = an.newest > asked ? an.newest : 0;
    if (an.pend) an.snap = WT.trace;
    advance();
    if (an.pk) an.next = R; else { R.t0 = now(); an.pk = R; }
  }
  // The packet on its way is over: its round's result stays in the readout, the waiting packet starts.
  function advance() {
    var t = now();
    while (an.pk && t - an.pk.t0 >= an.pk.end * 1000) {
      an.last = an.pk;
      an.pk = an.next; an.next = null;
      if (an.pk) an.pk.t0 = t;
    }
  }

  // ---------- frame loop: only while a packet moves or a cross / a reply waits to go; at most 30 frames a second ----------
  function canRun() { return !hold && !!host && !!stage && stage.isConnected && an.vis && !document.hidden && !an.min && !!lastD && !!sc; }
  function loopAfter(ms) {
    if (an.raf || an.timer || !canRun()) return;
    an.timer = setTimeout(function () { an.timer = 0; an.raf = requestAnimationFrame(tick); }, Math.max(0, ms));
  }
  function stopLoop() {
    if (an.timer) clearTimeout(an.timer);
    if (an.raf) cancelAnimationFrame(an.raf);
    an.timer = an.raf = 0;
  }
  function tick(ts) {
    an.raf = 0;
    if (!canRun()) return;
    var wait = drawL1(lastD);
    ribCursor();
    if (wait < 0) return;
    loopAfter(wait - (now() - ts)); // counted from this frame's time: the next one comes 33 ms or more later
    checkMin();
  }
  // A minimised window may still count as visible (WebView2): in the app the window itself is asked, at most once a
  // second while a packet moves; while it is minimised the packet waits and the question repeats every second.
  function appWin() {
    try { var T = window.__TAURI__; return T && T.window && T.window.getCurrentWindow ? T.window.getCurrentWindow() : null; } catch (e) { return null; }
  }
  function checkMin() {
    var w = appWin(), t = Date.now();
    if (!w || !w.isMinimized || t - an.minAt < 1000) return;
    an.minAt = t;
    w.isMinimized().then(function (m) { if (m && !an.min) { an.min = true; stopLoop(); pollMin(); } }, function () { /* not allowed: the packet keeps going */ });
  }
  function pollMin() {
    if (an.minPoll) return;
    an.minPoll = setTimeout(function () {
      an.minPoll = 0;
      var w = appWin();
      if (!an.min || !w) { an.min = false; return; }
      w.isMinimized().then(function (m) { if (m) pollMin(); else { an.min = false; schedule(); } }, function () { an.min = false; schedule(); });
    }, 1000);
  }

  // ---------- side column: the selected hop and the ribbon ----------
  function mk(key) {
    if (key === 'silent') {
      return '<svg class="mk" viewBox="0 0 10 10" aria-hidden="true" focusable="false"><rect width="10" height="10" rx="2" class="mk-bg"/><rect width="10" height="10" rx="2" fill="url(#trHatch)"/></svg>';
    }
    return '<i class="mk mk-' + esc(key) + '" aria-hidden="true"></i>';
  }
  function cell(k, v, u, s, cls) {
    return '<div><span class="mp-k">' + esc(k) + '</span><b class="mp-v mono' + (cls ? ' ' + cls : '') + '">' + esc(v) + (u ? '<small>' + esc(u) + '</small>' : '') + '</b>' +
      '<span class="mp-s">' + esc(s) + '</span></div>';
  }
  var measureCv = null;
  function mcx() { if (!measureCv) measureCv = document.createElement('canvas'); return measureCv.getContext('2d'); }
  // Physics against the measurement on one bar of RTT: grey = what light in fibre needs, blue = the extra; a tick at
  // the vacuum minimum. A reply faster than fibre allows gets a mark at the measured value instead of a bar.
  function gaugeHtml(g, meas) {
    var Wg = Math.max(220, Math.floor(panelEl.clientWidth || 420)), fib = g.fiberMs, vac = g.vacuumMs, c = mcx();
    var top = niceUp(Math.max(fib, meas, vac, 1) * 1.08, true), step = niceUp(top / 7), by = 19, s = '';
    function sx(v) { return Math.max(0, Math.min(Wg - 1, Math.round(v / top * (Wg - 1)))); }
    font(c, 12, SYS);
    var head = L('RTT, мс: физика и замер', 'RTT, ms: physics vs measured'), hw = c.measureText(head).width;
    s += '<text class="g-k" x="0" y="11">' + esc(head) + '</text>';
    if (meas > fib) {
      var ex = L('лишние ', 'extra ') + dec(meas - fib) + NB + L('мс', 'ms'), exw = c.measureText(ex).width;
      s += '<text class="g-x" x="' + Math.min(Wg, Math.max(sx(meas), hw + 12 + exw)) + '" y="11" text-anchor="end">' + esc(ex) + '</text>';
    }
    s += '<rect class="g-fib" x="0" y="' + by + '" width="' + sx(fib) + '" height="12"/>';
    if (meas > fib) s += '<rect class="g-ext" x="' + sx(fib) + '" y="' + by + '" width="' + Math.max(1, sx(meas) - sx(fib)) + '" height="12"/>';
    else s += '<rect class="g-meas ' + (g.verdict === 'impossible' ? 'bad' : g.verdict === 'doubtful' ? 'doubt' : 'ok') + '" x="' + Math.max(0, sx(meas) - 1) + '" y="' + (by - 3) + '" width="2" height="18"/>';
    if (vac > 0) s += '<rect class="g-vac" x="' + Math.max(0, sx(vac) - 1) + '" y="' + by + '" width="2" height="12"/>';
    for (var v = 0; v <= top + 1e-9; v += step) s += '<rect class="g-tick" x="' + Math.min(sx(v), Wg - 1) + '" y="' + (by + 14) + '" width="1" height="3"/>';
    font(c, 11, MONO);
    var labs = [
      { v: meas, t: dec(meas) + ' ' + L('замер', 'measured'), cls: 'g-lm' },
      { v: fib, t: dec(fib) + ' ' + L('волокно', 'fibre'), cls: 'g-l' },
      { v: vac, t: dec(vac) + ' ' + L('вакуум', 'vacuum'), cls: 'g-l', mid: true },
      { v: 0, t: '0', cls: 'g-l0', one: true },
      { v: top, t: nf(top), cls: 'g-l0', end: true, one: true },
    ], rows = [[], []], used = 0;
    // The values first; the ends of the scale only where they fit on the first line.
    labs.forEach(function (lb) {
      var w = c.measureText(lb.t).width, x = sx(lb.v), x0 = lb.end ? x - w : lb.mid ? x - w / 2 : x;
      x0 = Math.max(0, Math.min(Wg - w, x0));
      for (var r = 0; r < (lb.one ? 1 : 2); r++) {
        if (rows[r].every(function (o) { return x0 + w + 6 <= o[0] || x0 >= o[1] + 6; })) {
          rows[r].push([x0, x0 + w]); used = Math.max(used, r);
          s += '<text class="' + lb.cls + '" x="' + x0.toFixed(1) + '" y="' + (by + 30 + r * 13) + '">' + esc(lb.t) + '</text>';
          return;
        }
      }
    });
    var Hg = by + 34 + used * 13;
    var aria = L('RTT: минимум по физике ', 'RTT: physical minimum ') + ms1(fib) + L(', измерено ', ', measured ') + ms1(meas) +
      (meas > fib ? L(', лишние ', ', extra ') + ms1(meas - fib) : '');
    return '<div class="mp-gauge"><svg width="' + Wg + '" height="' + Hg + '" viewBox="0 0 ' + Wg + ' ' + Hg + '" role="img" aria-label="' + esc(aria) + '">' + s + '</svg></div>';
  }
  function placeHtml(h, g, d) {
    if (!h.addr) return esc(L('Адреса нет: ', 'No address: ') + stWord(h.status, d.m) + L('. Место неизвестно.', '. The place is unknown.'));
    // Without a usable database the core still sends a place for every hop with an address: level none for those it
    // would look up. Own hops and special ranges keep their own text below.
    var st = d.geo && d.geo.state;
    if (!g || (g.level === 'none' && !g.special)) {
      if (st === 'missing') return esc(L('База мест не установлена.', 'The places database is not installed.'));
      if (st === 'unreadable') return esc(L('База мест не читается.', 'The places database cannot be read.'));
      if (!g) return esc(L('Места для этого адреса нет.', 'No place for this address.'));
    }
    if (g.level === 'own') {
      if (d.home) {
        return esc(L('Адрес вашей сети или сети провайдера (' + specialName(g.special) + '): база таких мест не знает, хоп показан у дома.',
          'An address of your network or your provider (' + specialName(g.special) + '): the database has no places for these, the hop is shown at home.'));
      }
      // No home (no public city before the target, none set by hand): own hops sit at the first known place.
      return esc(L('Адрес вашей сети или сети провайдера (' + specialName(g.special) + '): база таких мест не знает.', 'An address of your network or your provider (' +
        specialName(g.special) + '): the database has no places for these.') + (d.empty ? '' : ' ' + L('Дома на карте нет, хоп показан у первого известного места.',
        'There is no home on the map, the hop is shown at the first known place.')));
    }
    if (g.level === 'none') {
      return esc(g.special ? L('Адрес из особого диапазона (' + specialName(g.special) + '): место не ищется.', 'An address from a special range (' + specialName(g.special) + '): no place is looked up.')
        : L('В базе нет места для этого адреса.', 'The database has no place for this address.'));
    }
    var s = esc(L('Место по базе: ', 'Database place: ')) + '<b>' + esc(placeName(g)) + '</b>';
    if (g.level === 'country') s += esc(L(', известна только страна. Проверке скоростью света нужен город.', ', only the country is known. The speed-of-light check needs a city.'));
    else {
      s += esc(L(', город. ', ', city. '));
      if (g.verdict === 'ok') s += esc(L('Проверка скоростью света пройдена.', 'Speed-of-light check passed.'));
      else if (g.verdict === 'doubtful') s += esc(L('Ответ пришел быстрее, чем свет прошел бы туда и обратно по волокну: место сомнительно.', 'The reply came faster than light could go there and back through fibre: the place is doubtful.'));
      else if (g.verdict === 'impossible') {
        s += esc(L('Свет не успел бы дойти туда и обратно за ' + ms1(g.rttMs) + ': место в базе неверно' + (h.dest ? ', цель, вероятно, anycast.' : '.'),
          'Light could not get there and back in ' + ms1(g.rttMs) + ': the database place is wrong' + (h.dest ? ', the destination is likely anycast.' : '.')));
      } else if (!d.home) s += esc(L('Дом неизвестен, проверять не от чего.', 'The home is unknown, nothing to check against.'));
      else s += esc(L('Ответов нет, проверять нечем.', 'No replies to check with.'));
    }
    if (fin(g.lat) && fin(g.lon)) s += '<br><span class="mono">' + esc(latT(g.lat, 2) + NB + NB + NB + lonT(g.lon, 2)) + '</span>';
    return s;
  }
  function panelHtml(d) {
    var h = d.byTtl[d.sel];
    if (!h) return '';
    var m = d.m, g = h.geo || null, nm = nameOf(h), stk = h.origin || h.status === 'loss' ? 'loss' : h.status === 'partial' ? 'partial' : 'other';
    var out = '<div class="mp-ph"><span class="mp-ttl mono">' + esc(h.ttl) + '</span><div class="mp-id">' +
      '<div class="mp-name mono">' + esc(nm || h.addr || L('адрес неизвестен', 'address unknown')) + '</div>' +
      (nm && h.addr ? '<div class="mp-addr mono">' + esc(h.addr) + '</div>' : '') + '</div>' +
      '<span class="mp-state s-' + stk + '">' + mk(h.status) + esc(stOf(h, m)) + '</span></div>';
    var cells = [], phys = !!(g && fin(g.distKm) && fin(g.fiberMs) && fin(g.vacuumMs));
    if (phys) {
      cells.push(cell(L('От старта', 'From start'), thou(g.distKm), L('км', 'km'),
        d.manual ? L('(дом задан вручную)', '(home set by hand)') : L('от ' + (d.home && d.home.city || 'дома') + ', по кратчайшей', 'from ' + (d.home && d.home.city || 'home') + ', great circle')));
      cells.push(cell(L('Минимум по физике', 'Physical minimum'), dec(g.fiberMs), L('мс', 'ms'), L('волокно; ', 'fibre; ') + ms1(g.vacuumMs) + L(' вакуум', ' vacuum')));
    }
    var meas = g && fin(g.rttMs) ? g.rttMs : h.full && fin(h.full.min) ? h.full.min : null;
    if (meas != null) cells.push(cell(L('Измерено', 'Measured'), dec(meas), L('мс', 'ms'), L('мин. RTT за ', 'min RTT over ') + winOf(m)));
    if (phys && g.verdict === 'ok' && fin(g.extraMs)) cells.push(cell(L('Лишняя задержка', 'Extra delay'), '+' + dec(g.extraMs), L('мс', 'ms'), L('сверх волокна', 'above fibre')));
    else if (phys && (g.verdict === 'impossible' || g.verdict === 'doubtful')) {
      var bad = g.verdict === 'impossible';
      cells.push(cell(L('Проверка', 'Check'), bad ? L('невозможно', 'impossible') : L('сомнительно', 'doubtful'), '',
        bad ? L('быстрее света', 'faster than light') : L('быстрее, чем по волокну', 'faster than fibre'), bad ? 't-loss' : 't-part'));
    }
    if (h.s) {
      var sent = h.s.sent != null ? h.s.sent : m.rounds, nr = h.status === 'partial' && h.s.noReplyPct > 0;
      cells.push(cell(L('Потери', 'Loss'), dec(h.s.lossPct), '%', L('из ' + thou(sent) + ' проб', 'of ' + thou(sent) + ' probes') +
        (nr ? L(', без ответа ' + pctT(h.s.noReplyPct), ', ' + pctT(h.s.noReplyPct) + ' unanswered') : ''), h.s.lossPct > 0 ? 't-loss' : ''));
    }
    if (g && (g.asn || g.cc)) {
      cells.push(g.asn ? cell(L('AS, страна', 'AS, country'), 'AS' + g.asn, '', [g.asOrg, g.cc].filter(Boolean).join(', '))
        : cell(L('Страна', 'Country'), g.cc, '', g.country || ''));
    }
    out += '<div class="mp-read">' + cells.join('') + '</div>';
    if (phys && meas != null) out += gaugeHtml(g, meas);
    return out + '<div class="mp-place">' + placeHtml(h, g, d) + '</div>';
  }
  function glyph(st) {
    var a = '<svg class="mp-g" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" focusable="false">';
    if (st === 'loss') return a + '<rect class="f-loss" width="10" height="10"/></svg>';
    if (st === 'partial') return a + '<rect class="f-part" width="2" height="10"/><rect class="f-part" x="4" width="2" height="10"/><rect class="f-part" x="8" width="2" height="10"/></svg>';
    if (st === 'silent') return a + '<path class="s-hatch" d="M-2 4 L4 -2 M-2 10 L10 -2 M4 12 L12 4"/></svg>';
    if (st === 'noreply' || st === 'unreachable') return a + '<rect class="f-mute" x="1" y="1" width="8" height="8" rx="1.5"/></svg>';
    return '';
  }
  // The ribbon: mtr-like lines (min RTT and loss over the window or the picked period, as the table shows them).
  // Built once and then patched in place (text, classes and attributes of the same nodes), so a click, the focus, the
  // scroll and a scrollbar drag survive live data and the packet's cursor. The status words are also the row's title:
  // a narrow column shows only the status mark.
  var rib = null;
  function setText(el, t) { if (el.textContent !== t) el.textContent = t; }
  function setAttr(el, k, v) { if (el.getAttribute(k) !== v) el.setAttribute(k, v); }
  function ribBuild() {
    ribEl.innerHTML = '<div class="mp-rh"><h3>' + esc(L('Лента хопов', 'Hops')) + '</h3><span></span></div>' +
      '<div class="mp-rows" role="listbox" aria-label="' + esc(L('Хопы', 'Hops')) + '"><div class="mp-row hd" aria-hidden="true"><span class="n">#</span><span>' + esc(L('адрес', 'address')) +
      '</span><span class="r">' + esc(L('мин', 'min')) + '</span><span class="l">' + esc(L('потери', 'loss')) + '</span><span class="g"></span><span class="s">' + esc(L('статус', 'status')) + '</span></div></div>';
    rib = { head: ribEl.querySelector('.mp-rh span'), box: ribEl.querySelector('.mp-rows'), list: [] };
  }
  // The packet's cursor on the hop it is at (lost: where a lost round ends).
  function ribCursor() {
    if (!rib) return;
    rib.list.forEach(function (r) {
      var t = +r.getAttribute('data-mp-ttl');
      setAttr(r.firstChild.firstChild, 'class', cursor.ttl === t ? (cursor.lost ? 'lost' : 'cur') : '');
    });
  }
  function ribPatch(d) {
    if (!rib) ribBuild();
    var m = d.m;
    setText(rib.head, m.range ? L('мин. RTT и потери за выделенный период', 'min RTT and loss over the selected period') : L('мин. RTT и потери за ', 'min RTT and loss over ') + winOf(m));
    while (rib.list.length < d.hops.length) {
      var nr = document.createElement('div');
      nr.setAttribute('role', 'option');
      nr.innerHTML = '<span class="n"><b></b></span><span class="a"></span><span class="r"></span><span class="l"></span><span class="g"></span><span class="s"></span>';
      rib.box.appendChild(nr);
      rib.list.push(nr);
    }
    while (rib.list.length > d.hops.length) rib.box.removeChild(rib.list.pop());
    d.hops.forEach(function (h, i) {
      var r = rib.list[i], c = r.children, sel = h.ttl === d.sel, s = h.s, st = stOf(h, m);
      var k = h.origin || h.status === 'loss' ? 'loss' : h.status === 'partial' ? 'partial' : 'other';
      setAttr(r, 'class', 'mp-row' + (sel ? ' sel' : ''));
      setAttr(r, 'data-mp-ttl', String(h.ttl));
      setAttr(r, 'aria-selected', String(sel));
      setAttr(r, 'tabindex', sel ? '0' : '-1');
      setAttr(r, 'title', st);
      setText(c[0].firstChild, String(h.ttl));
      setText(c[1], h.addr || '*');
      setText(c[2], s && fin(s.min) ? nf(s.min) : '*');
      setText(c[3], s ? pctT(s.lossPct) : '*');
      if (r.getAttribute('data-mp-g') !== h.status) { c[4].innerHTML = glyph(h.status); r.setAttribute('data-mp-g', h.status); }
      setAttr(c[5], 'class', 's s-' + k);
      setText(c[5], st);
    });
    ribCursor();
  }
  function renderSide(d) {
    var ph = panelHtml(d);
    if (ph !== keys.panel) { panelEl.innerHTML = ph; keys.panel = ph; }
    var selChanged = keys.sel !== d.sel, had = ribEl.contains(document.activeElement);
    keys.sel = d.sel;
    ribPatch(d);
    if (selChanged) {
      var box = rib.box, row = ribEl.querySelector('.mp-row.sel'), hd = ribEl.querySelector('.mp-row.hd');
      // keys in the ribbon move the selection: the focus goes along
      if (row && had && document.activeElement !== row) row.focus({ preventScroll: true });
      if (box && row) {
        var top0 = row.offsetTop - (hd ? hd.offsetHeight : 0), bottom = row.offsetTop + row.offsetHeight;
        if (top0 < box.scrollTop) box.scrollTop = top0;
        else if (bottom > box.scrollTop + box.clientHeight) box.scrollTop = bottom - box.clientHeight;
      }
    }
  }

  // ---------- pointer and keys ----------
  function local(e) { var r = mapEl.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; }
  // Badges first, then a callout plate, then a marker (a click on the selected hop's group goes to its next hop).
  function hitAt(x, y) {
    var best = null;
    hits.forEach(function (t) { if (inBox(x, y, t) && (!best || t.k < best.k)) best = t; });
    if (!best) return null;
    if (best.k !== 2) return best.ttl;
    var ttls = best.g.ttls.filter(function (t) { return t > 0; }), i = ttls.indexOf(input && input.m ? input.m.selected : null);
    return i >= 0 ? ttls[(i + 1) % ttls.length] : ttls[0];
  }
  function onMapClick(e) {
    if (!menuEl.hidden || !sc) return;
    var p = local(e), t = hitAt(p[0], p[1]);
    if (t != null && opt.onSelect) opt.onSelect(t);
  }
  function onMapMove(e) {
    var p = local(e);
    mapEl.style.cursor = sc && hitAt(p[0], p[1]) != null ? 'pointer' : '';
  }
  // Right click: «Дом здесь» (the point rounded to 0.1 degree) and «Сбросить дом» when the home was set by hand.
  function openMenu(x, y) {
    if (!sc || !input) return;
    var ll = G.inv(sc.proj, [x, y]);
    if (!ll) return;
    menuLL = { lat: Math.round(ll[1] * 10) / 10, lon: Math.round(G.norm(ll[0]) * 10) / 10 };
    var manual = !!(input.home && input.home.source === 'manual');
    menuEl.innerHTML = '<button type="button" role="menuitem" data-mp-act="home">' + esc(L('Дом здесь', 'Home here')) +
      '<span class="aside">' + esc(latT(menuLL.lat, 1) + ' ' + lonT(menuLL.lon, 1)) + '</span></button>' +
      (manual ? '<button type="button" role="menuitem" data-mp-act="unhome">' + esc(L('Сбросить дом', 'Reset home')) + '</button>' : '');
    menuEl.hidden = false;
    var mw = menuEl.offsetWidth, mh = menuEl.offsetHeight;
    menuEl.style.left = Math.max(4, Math.min(x, W - mw - 4)) + 'px';
    menuEl.style.top = Math.max(4, Math.min(y, H - mh - 4)) + 'px';
    window.addEventListener('keydown', onEscMenu, true);
    document.addEventListener('pointerdown', onOutside, true);
    menuEl.firstChild.focus();
  }
  function closeMenu(refocus) {
    if (!menuEl || menuEl.hidden) return;
    menuEl.hidden = true;
    menuEl.innerHTML = '';
    window.removeEventListener('keydown', onEscMenu, true);
    document.removeEventListener('pointerdown', onOutside, true);
    if (refocus) mapEl.focus({ preventScroll: true });
  }
  // The open menu takes Esc before the screen (full screen, the picked period).
  function onEscMenu(e) {
    if (e.key !== 'Escape') return;
    e.preventDefault(); e.stopPropagation();
    closeMenu(true);
  }
  function onOutside(e) { if (!menuEl.contains(e.target)) closeMenu(false); }
  function onContext(e) {
    if (!sc) return;
    e.preventDefault();
    var p = local(e);
    openMenu(p[0], p[1]);
  }
  function onMapKey(e) {
    if (e.target !== mapEl) return;
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) { e.preventDefault(); openMenu(W / 2, H / 2); }
  }
  function onMenuClick(e) {
    var b = e.target.closest && e.target.closest('[data-mp-act]');
    if (!b) return;
    var a = b.getAttribute('data-mp-act'), ll = menuLL;
    closeMenu(true);
    if (opt.onHome) opt.onHome(a === 'home' ? ll : null);
  }
  function onMenuKey(e) {
    var items = [].slice.call(menuEl.querySelectorAll('[role="menuitem"]')), i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      var j = i + (e.key === 'ArrowDown' ? 1 : -1);
      items[(j + items.length) % items.length].focus();
    } else if (e.key === 'Tab') closeMenu(false);
  }
  function onRibClick(e) {
    var r = e.target.closest && e.target.closest('[data-mp-ttl]');
    if (r && opt.onSelect) opt.onSelect(+r.getAttribute('data-mp-ttl'));
  }
  function onRibKey(e) {
    var r = e.target.closest && e.target.closest('[data-mp-ttl]');
    if (!r) return;
    var rows = [].slice.call(ribEl.querySelectorAll('[data-mp-ttl]')), i = rows.indexOf(r);
    var j = e.key === 'ArrowDown' ? i + 1 : e.key === 'ArrowUp' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? rows.length - 1 : -2;
    if (j === -2 || j < 0 || j >= rows.length) return;
    e.preventDefault();
    if (opt.onSelect) opt.onSelect(+rows[j].getAttribute('data-mp-ttl'));
  }

  // ---------- the picture: for publication (always dark, your network hidden) and the report's map (light, as is) ----------
  // MAP.png(data | null, { size: 'wide' | 'square', theme: 'dark' | 'light', mask }) draws at 2× of 1200×675 or
  // 1080×1080: a header line (the target and the window), the map as on screen with its callouts, legend and scale,
  // without the packet and its readout, the hop ribbon (right of the map; under it in the square) and the sources.
  // data: {m, geo, home} as MAP.update takes it (null: the map's own). Returns a data: URL of the PNG, or null when
  // the route has no places. The live map's drawing state is put aside while the picture is drawn, then comes back.
  var SIZES = { wide: [1200, 675], square: [1080, 1080] };
  // The colours of a theme from the tokens, read on a hidden probe with that data-theme.
  function themeColors(theme) {
    var el = document.createElement('div'), o = {};
    el.setAttribute('data-theme', theme);
    el.style.cssText = 'position:absolute;left:-9999px;top:0;visibility:hidden';
    document.body.appendChild(el);
    var cs = getComputedStyle(el);
    TOK.forEach(function (k) { o[k] = cs.getPropertyValue('--' + k).trim(); });
    el.parentNode.removeChild(el);
    return o;
  }
  // For publication: own hops (your network, the provider's CGNAT) and the first public hops until the AS number
  // changes (a hop without one does not change it; without AS data: the first two) lose their address, name and
  // place; they and the home go to the home country's label point (trace.home.cc; a home set by hand has none: the
  // country of those provider hops; without one, the first place left), so the home callout says only «ваша сеть».
  // The run of public hops ends at the destination: the target is in the picture's header anyway, and a short route
  // keeps something to show; a destination in the provider's AS (its DNS, say) keeps its address but only its
  // country. Callouts leave out the distances from home too (two of them would point at it).
  function masked(inp) {
    var hops = inp.m.hops || [], hide = {}, first = null, k = 0, CP = window.WT_COUNTRY_POINTS || {};
    hops.forEach(function (x) { if (x.geo && x.geo.level === 'own') hide[x.ttl] = true; });
    for (var i = 0; i < hops.length; i++) {
      var h = hops[i];
      if (!h.addr || hide[h.ttl]) continue;
      if (h.dest) break;
      var asn = h.geo && h.geo.asn != null ? h.geo.asn : null;
      if (first == null ? k >= 2 : asn != null && asn !== first) break;
      if (first == null) first = asn;
      hide[h.ttl] = true; k++;
    }
    var own = { level: 'own', special: null, city: null, region: null, cc: null, country: null, lat: null, lon: null, asn: null, asOrg: null,
      distKm: null, fiberMs: null, vacuumMs: null, rttMs: null, extraMs: null, verdict: null };
    var dest = hops.filter(function (x) { return x.dest && !hide[x.ttl] && x.addr && x.geo && first != null && x.geo.asn === first; })[0] || null;
    var dpt = dest && dest.geo.cc ? CP[dest.geo.cc] || null : null;
    var out = hops.map(function (x) {
      if (hide[x.ttl]) return Object.assign({}, x, { addr: L('скрыт', 'hidden'), name: null, masked: true, geo: own });
      if (x !== dest) return x;
      return Object.assign({}, x, { name: null, geo: Object.assign({}, own, dpt ? { level: 'country', cc: dest.geo.cc, country: dest.geo.country || null, lat: dpt[1], lon: dpt[0] } : { level: 'none' }) });
    });
    var home = inp.home, cc = home && home.cc;
    if (!cc) hops.forEach(function (x) { if (!cc && hide[x.ttl] && x.geo && x.geo.cc) cc = x.geo.cc; });
    var pt = cc ? CP[cc] || null : null;
    if (!pt) {
      var f = G.kinds(out.filter(function (x) { return !x.masked; }), null, window.WT_COUNTRY_POINTS).filter(function (n) { return n.kind === 'city' || n.kind === 'country'; })[0];
      pt = f ? f.ll : null;
    }
    return {
      m: Object.assign({}, inp.m, { hops: out }), geo: inp.geo,
      home: home && pt ? { lat: pt[1], lon: pt[0], city: null, cc: cc || null, country: null, source: 'auto', ttl: null } : null,
    };
  }
  function ctx2d(w, h, k) {
    var c = document.createElement('canvas');
    c.width = Math.round(w * k); c.height = Math.round(h * k);
    var g = c.getContext('2d');
    g.setTransform(k, 0, 0, k, 0, 0);
    return g;
  }
  function rrect(g, x, y, w, h, r) {
    g.beginPath(); g.moveTo(x + r, y);
    g.arcTo(x + w, y, x + w, y + h, r); g.arcTo(x + w, y + h, x, y + h, r);
    g.arcTo(x, y + h, x, y, r); g.arcTo(x, y, x + w, y, r); g.closePath();
  }
  // A text cut to a width with an ellipsis.
  function fitText(g, s, w) {
    s = String(s);
    if (g.measureText(s).width <= w) return s;
    while (s.length > 1 && g.measureText(s + '…').width > w) s = s.slice(0, -1);
    return s + '…';
  }
  // The status mark of a ribbon line, 10×10 at (x, y), as on screen.
  function glyphPic(g, st, x, y) {
    if (st === 'loss') { g.fillStyle = C.loss; g.fillRect(x, y, 10, 10); }
    else if (st === 'partial') { g.fillStyle = C.partial; [0, 4, 8].forEach(function (k) { g.fillRect(x + k, y, 2, 10); }); }
    else if (st === 'silent') {
      g.save(); g.beginPath(); g.rect(x, y, 10, 10); g.clip();
      g.strokeStyle = C.hatch; g.lineWidth = 1.6; g.beginPath();
      g.moveTo(x - 2, y + 4); g.lineTo(x + 4, y - 2); g.moveTo(x - 2, y + 10); g.lineTo(x + 10, y - 2); g.moveTo(x + 4, y + 12); g.lineTo(x + 12, y + 4);
      g.stroke(); g.restore();
    } else if (st === 'noreply' || st === 'unreachable') { g.fillStyle = C.muted; rrect(g, x + 1, y + 1, 8, 8, 1.5); g.fill(); }
  }
  var RIB_TOP = 12 + 24 + 22, RIB_PAD = 10; // the ribbon's head and column names, its bottom padding
  function hopsRu(n) { var a = n % 10, b = n % 100; return a === 1 && b !== 11 ? 'хоп' : a >= 2 && a <= 4 && (b < 12 || b > 14) ? 'хопа' : 'хопов'; }
  function ribNeed(n, cols, rowH) { return RIB_TOP + Math.ceil(n / cols) * rowH + RIB_PAD; }
  // The hop ribbon of the picture: ttl, address (a hidden one in grey), min RTT and loss over the window or the picked
  // period, the status mark and words; a long route in two columns or in lower lines, what does not fit is counted.
  function ribbonPic(g, d, R) {
    var m = d.m, hs = d.hops, n = hs.length, px = 12, inner = R.w - 2 * px, avail = R.h - RIB_TOP - RIB_PAD;
    g.fillStyle = C.well; rrect(g, R.x, R.y, R.w, R.h, 8); g.fill();
    g.textBaseline = 'middle'; g.textAlign = 'left';
    var title = L('Лента хопов', 'Hops'), ty = R.y + 12 + 11;
    font(g, 13, SYS, 600); g.fillStyle = C.title; g.fillText(title, R.x + px, ty);
    var tw = g.measureText(title).width;
    font(g, 12, SYS); g.fillStyle = C.muted; g.textAlign = 'right';
    g.fillText(fitText(g, m.range ? L('мин. RTT и потери за выделенный период', 'min RTT and loss over the selected period') : L('мин. RTT и потери за ', 'min RTT and loss over ') + winOf(m), inner - tw - 12), R.x + R.w - px, ty);
    g.textAlign = 'left';
    font(g, 12, MONO);
    var ch = g.measureText('0').width, need = 33.5 * ch, cols = n * 22 > avail && inner >= 2 * need + 24 + 2 * 60 ? 2 : 1;
    var colW = (inner - (cols - 1) * 24) / cols, per = Math.ceil(n / cols), rowH = Math.max(12, Math.min(22, Math.floor(avail / Math.max(1, per))));
    var fit = Math.floor(avail / rowH), words = colW - need >= 60;
    for (var col = 0; col < cols; col++) {
      var x = R.x + px + col * (colW + 24), y = R.y + RIB_TOP - 11;
      // column names
      font(g, 12, MONO); g.fillStyle = C.muted; g.textAlign = 'right';
      g.fillText('#', x + 2 * ch, y); g.fillText(L('мин', 'min'), x + 23.5 * ch, y); g.fillText(L('потери', 'loss'), x + 30.5 * ch, y);
      g.textAlign = 'left'; g.fillText(L('адрес', 'address'), x + 3 * ch, y);
      if (words) { font(g, 12, SYS); g.fillText(L('статус', 'status'), x + need + 4, y); }
      var list = hs.slice(col * per, col * per + per), more = 0;
      if (list.length > fit) { more = list.length - (fit - 1); list = list.slice(0, fit - 1).concat([null]); }
      list.forEach(function (h, i) {
        var cy = R.y + RIB_TOP + i * rowH + rowH / 2;
        font(g, 12, MONO); g.textAlign = 'left';
        if (!h) { g.fillStyle = C.muted; g.fillText(L('и еще ' + more + ' ' + hopsRu(more), 'and ' + more + ' more ' + (more === 1 ? 'hop' : 'hops')), x + 3 * ch, cy); return; }
        var s = h.s, k = h.origin || h.status === 'loss' ? 'loss' : h.status === 'partial' ? 'partial' : 'other';
        g.fillStyle = C.text; g.textAlign = 'right';
        g.fillText(String(h.ttl), x + 2 * ch, cy);
        g.fillText(s && fin(s.min) ? nf(s.min) : '*', x + 23.5 * ch, cy);
        g.fillText(s ? pctT(s.lossPct) : '*', x + 30.5 * ch, cy);
        g.textAlign = 'left'; g.fillStyle = h.masked ? C.muted : C.text;
        g.fillText(fitText(g, h.addr || '*', 15.3 * ch), x + 3 * ch, cy);
        glyphPic(g, h.status, Math.round(x + 32 * ch - 5), Math.round(cy - 5));
        if (words) {
          font(g, 12, SYS); g.fillStyle = k === 'loss' ? C['loss-text'] : k === 'partial' ? C['partial-text'] : C.text;
          g.fillText(fitText(g, stOf(h, m), colW - need - 4), x + need + 4, cy);
        }
      });
    }
  }
  function png(data, o) {
    o = o || {};
    var inp = data || input;
    if (!inp || !inp.m || inp.m.kind !== 'trace') return null;
    var src = o.mask ? masked(inp) : inp;
    var d = derive({ m: Object.assign({}, src.m, { selected: null }), geo: src.geo, home: src.home });
    if (d.empty) return null;
    d.mask = !!o.mask;
    var sz = SIZES[o.size] || SIZES.wide, PW = sz[0], PH = sz[1], K = 2, pad = 24, top = 48, foot = 32, gap = 16, bodyH = PH - top - foot, mapR, ribR;
    if (PW > PH) {
      // the ribbon as wide as the status words of its usual lines need (the app's 448 px column at 1320 px)
      mapR = { x: pad, y: top, w: PW - 2 * pad - 412 - gap, h: bodyH };
      ribR = { x: pad + mapR.w + gap, y: top, w: 412, h: bodyH };
    } else {
      var cap = Math.round(bodyH * 0.4), rh = ribNeed(d.hops.length, 1, 22);
      if (rh > cap) rh = Math.min(cap, ribNeed(d.hops.length, 2, 22));
      mapR = { x: pad, y: top, w: PW - 2 * pad, h: bodyH - rh - gap };
      ribR = { x: pad, y: top + mapR.h + gap, w: PW - 2 * pad, h: rh };
    }
    var keep = { W: W, H: H, DPR: DPR, C: C, sc: sc, fitPrev: fitPrev, choicePrev: choicePrev, hits: hits, cx: cx, cache: cache, cacheCx: cacheCx };
    var out = document.createElement('canvas');
    out.width = PW * K; out.height = PH * K;
    try {
      XP = true;
      C = themeColors(o.theme === 'light' ? 'light' : 'dark');
      W = mapR.w; H = mapR.h; DPR = K; fitPrev = null; choicePrev = {}; hits = []; sc = null;
      cx = [ctx2d(W, H, K), null, ctx2d(W, H, K)];
      cacheCx = ctx2d(W, H, K); cache = cacheCx.canvas;
      layout(d, 'png' + PW + 'x' + PH, 'png');
      drawCache(); drawL0(d); drawL2(d);
      var g = out.getContext('2d'), m = d.m, tg = m.target || {}, x = pad;
      g.setTransform(K, 0, 0, K, 0, 0);
      g.fillStyle = C.layer; g.fillRect(0, 0, PW, PH);
      // the target and the window
      g.textBaseline = 'middle'; g.textAlign = 'left';
      font(g, 16, MONO, 600); g.fillStyle = C.title; g.fillText(tg.host || '', x, 25);
      x += g.measureText(tg.host || '').width + 12;
      font(g, 13, SYS);
      if (tg.name && tg.name !== tg.host) { g.fillStyle = C.text; g.fillText(tg.name, x, 25); x += g.measureText(tg.name).width + 12; }
      g.fillStyle = C.muted;
      g.fillText(m.range ? L(probeName(m) + ', выделенный период', probeName(m) + ', selected period') : L(probeName(m) + ', окно ' + winOf(m), probeName(m) + ', ' + winOf(m) + ' window'), x, 25);
      // the map: its well, then the land, route and markers, then the callouts, legend and scale
      g.save(); rrect(g, mapR.x, mapR.y, mapR.w, mapR.h, 8); g.clip();
      g.fillStyle = C.well; g.fillRect(mapR.x, mapR.y, mapR.w, mapR.h);
      g.drawImage(cx[0].canvas, mapR.x, mapR.y, mapR.w, mapR.h);
      g.drawImage(cx[2].canvas, mapR.x, mapR.y, mapR.w, mapR.h);
      g.restore();
      ribbonPic(g, d, ribR);
      // the sources
      var sim = d.geo && d.geo.state === 'simulated';
      font(g, 12, SYS); g.fillStyle = C.muted; g.textAlign = 'left'; g.textBaseline = 'middle';
      g.fillText('Whilo Trace · ' + (sim ? L('места: имитация', 'places: simulated') : 'IP Geolocation by DB-IP (db-ip.com), CC BY 4.0') + ' · ' +
        L('суша: Natural Earth', 'land: Natural Earth'), pad, PH - foot / 2);
    } finally {
      XP = false;
      W = keep.W; H = keep.H; DPR = keep.DPR; C = keep.C; sc = keep.sc; fitPrev = keep.fitPrev; choicePrev = keep.choicePrev; hits = keep.hits;
      cx = keep.cx; cache = keep.cache; cacheCx = keep.cacheCx;
    }
    return out.toDataURL('image/png');
  }
  // Whether the route of data (null: the map's own) has any place to draw.
  function hasPlaces(data) {
    var inp = data || input;
    return !!(inp && inp.m && inp.m.kind === 'trace' && !derive(inp).empty);
  }

  // ---------- public ----------
  function init(o) {
    opt = o || {};
    // Theme: no event fires when it changes (base/theme.js), so the map watches the attribute and the system theme.
    if (window.MutationObserver) new MutationObserver(schedule).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    onMq(DARK, schedule);
    // A hidden window (another tab in front; minimised, where the system says so): the packet waits until it is back.
    document.addEventListener('visibilitychange', function () { if (document.hidden) stopLoop(); else schedule(); });
    // The system's reduced-motion setting changed while the page is open: the map follows it unless the button was
    // used since; the screen hears it (onPause) to change the button.
    onMq(CALM, function () {
      var on = !!(CALM && CALM.matches);
      if (on === paused || (!on && !calm)) return;
      paused = on; calm = on;
      if (on) quiet();
      keys.l1 = ''; draw();
      if (opt.onPause) opt.onPause(on);
    });
  }
  // The hop card's host while the card shows «Карта», or null: the stage (canvases, caches) moves along; without a
  // host no packet runs.
  function attach(h) {
    if (h === host) return;
    if (host && stage && stage.parentNode === host) host.removeChild(stage);
    host = h || null;
    if (!host) {
      closeMenu(false); observe(false); quiet(); lastD = null;
      if (an.minPoll) { clearTimeout(an.minPoll); an.minPoll = 0; }
      an.min = false;
      return;
    }
    if (!stage) build();
    host.appendChild(stage);
    observe(true);
  }
  function update(data) { input = data; draw(); }
  // Test hook: the frame t seconds into a round of the current data, frozen (lostAt: the ttl where that round is lost,
  // 0: it reaches the target; omitted: as the data says), or the static route (t null). Live rounds stay off after it.
  function renderAt(t, lostAt) {
    hold = true;
    frozen = t == null ? null : +t;
    demoLost = lostAt == null ? undefined : +lostAt || null;
    quiet();
    keys.l1 = '';
    draw();
    return true;
  }
  window.MAP = {
    init: init, attach: attach, update: update, renderAt: renderAt, png: png, hasPlaces: hasPlaces,
    // A round of the traced target finished: its packet starts with the first snapshot that has it.
    round: onRound,
    setPaused: function (on) {
      paused = !!on; calm = false;
      if (paused) quiet();
      keys.l1 = ''; draw();
    },
    paused: function () { return paused; },
  };
})();
