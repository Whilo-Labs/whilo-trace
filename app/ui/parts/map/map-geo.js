// Whilo Trace, route map: where every hop is drawn (no drawing here; parts/map/map.js draws). Input: the hops of the
// trace part's model (ttl, addr, dest, status, origin, s, full, geo) and the trace's home, as docs/ui-contract.md
// describes them (Trace.home, hops[].geo). Steps:
//   kinds()  how sure each hop's place is, and the places known before the projection (lon, lat);
//   fit()    the projection: Equal Earth turned to the route, steady while live data ticks;
//   place()  screen points: unknown hops between their neighbours, groups of markers that touch, route lines,
//            the pointer to an impossible destination and the clock of the packet.
// The kind of a hop comes from geo.level and geo.verdict only (geo.special is set only for hops that were not looked up).
(function () {
  'use strict';
  var RING = 13; // radius of the country ring, px
  // Marker radius for grouping, which marker a group shows (the surest place first) and how far the line stays off it.
  var RAD = { home: 8, own: 8, city: 4, country: RING, impossible: 8, unknown: 8, anchored: 4 };
  var RANK = { home: 0, own: 0, city: 1, country: 2, impossible: 3, unknown: 4, anchored: 5 };
  var GAP = { home: 8, own: 8, city: 5, country: RING + 3, impossible: 9, unknown: 9, anchored: 5 };

  function fin(v) { return v != null && isFinite(v); }
  function norm(lon) { return ((lon % 360) + 540) % 360 - 180; }
  function dist(a, b) { var x = a[0] - b[0], y = a[1] - b[1]; return Math.sqrt(x * x + y * y); }
  function sameP(a, b) { return !!a && !!b && dist(a, b) < 0.5; }
  function okP(p) { return !!p && isFinite(p[0]) && isFinite(p[1]); }
  // [lon, lat] under a screen point, or null outside the world (Equal Earth's inverse answers there too: the point
  // must project back to where it came from).
  function inv(proj, p) {
    var q = proj.invert(p);
    if (!q || !isFinite(q[0]) || !isFinite(q[1]) || Math.abs(q[1]) > 90) return null;
    var b = proj(q);
    return okP(b) && dist(b, p) < 1 ? q : null;
  }

  // ---------- kinds and places on the globe ----------
  // One node per hop in TTL order, the home first (ttl 0) when it is known. kind:
  //   home        the home point (start of the route; the automatic home is a city on the path, never the target)
  //   own         an address of your network or your provider: drawn at home
  //   city        a city that passed the speed-of-light check (doubtful ones too, flagged)
  //   country     only the country is known: a ring at its point (the database point or the country's label point)
  //   impossible  light cannot reach the database point in the measured time: drawn between its neighbours
  //   unknown     no place (no answer, no record, a special range): drawn between its neighbours
  //   anchored    the destination with an impossible place (likely anycast): drawn at the last hop with a place,
  //               a dotted pointer goes towards its database point (db)
  // ll: [lon, lat] where the node is drawn, when known before the projection.
  function kinds(hops, home, countryPoints) {
    var nodes = [], hl = home && fin(home.lat) && fin(home.lon) ? [home.lon, home.lat] : null;
    if (hl) nodes.push({ ttl: 0, kind: 'home', ll: hl, hop: null });
    hops.forEach(function (h) {
      var g = h.geo, n = { ttl: h.ttl, hop: h, kind: 'unknown', ll: null, db: null, doubtful: false };
      if (g && h.addr) {
        var pt = fin(g.lat) && fin(g.lon) ? [g.lon, g.lat] : null;
        if (g.level === 'own') n.kind = 'own';
        else if (g.level === 'city' && pt) {
          if (g.verdict === 'impossible') { n.kind = h.dest ? 'anchored' : 'impossible'; n.db = pt; }
          else { n.kind = 'city'; n.ll = pt; n.doubtful = g.verdict === 'doubtful'; }
        } else if ((g.level === 'country' || g.level === 'city') && g.cc) {
          var cp = pt || (countryPoints && countryPoints[g.cc]) || null;
          if (cp) { n.kind = 'country'; n.ll = cp; }
        }
      }
      nodes.push(n);
    });
    // Own hops sit at home; with no home at all, at the first hop with a place, whose own marker the group keeps
    // there (noHome: the house would say that your network is in that city).
    var first = nodes.filter(function (n) { return n.kind === 'city' || n.kind === 'country'; })[0];
    var base = hl || (first ? first.ll : null);
    nodes.forEach(function (n, i) {
      if (n.kind === 'own') { n.ll = base; n.noHome = !hl; }
      if (n.kind === 'anchored') {
        for (var j = i - 1; j >= 0 && !n.ll; j--) if (nodes[j].kind === 'city' || nodes[j].kind === 'country') n.ll = nodes[j].ll;
        if (!n.ll) n.ll = hl;
      }
    });
    return nodes;
  }

  // ---------- projection ----------
  // Points the map must show: every node with a place and 8 samples on each arc between them (great circles bulge
  // towards the poles). Impossible database points stay out: the edge pointer shows their direction.
  function fitPoints(nodes) {
    var pts = [], last = null;
    nodes.forEach(function (n) {
      if (!n.ll) return;
      if (last && (last[0] !== n.ll[0] || last[1] !== n.ll[1])) {
        var f = d3.geoInterpolate(last, n.ll);
        for (var k = 1; k <= 8; k++) pts.push(f(k / 9));
      }
      pts.push(n.ll);
      last = n.ll;
    });
    return pts;
  }
  // Centre of the smallest longitude interval holding every point (a route over the Pacific is not cut in two).
  function lonCentre(pts) {
    var a = pts.map(function (p) { return norm(p[0]); }).sort(function (x, y) { return x - y; });
    if (!a.length) return 0;
    var gap = -1, at = 0;
    for (var i = 0; i < a.length; i++) {
      var g = (i + 1 < a.length ? a[i + 1] : a[0] + 360) - a[i];
      if (g > gap) { gap = g; at = i; }
    }
    var start = at + 1 < a.length ? a[at + 1] : a[0], end = a[at];
    if (end < start) end += 360;
    return norm((start + end) / 2);
  }
  function boundsOf(proj, pts) {
    var b = [Infinity, Infinity, -Infinity, -Infinity];
    pts.forEach(function (ll) {
      var p = proj(ll);
      if (!okP(p)) return;
      b[0] = Math.min(b[0], p[0]); b[1] = Math.min(b[1], p[1]); b[2] = Math.max(b[2], p[0]); b[3] = Math.max(b[3], p[1]);
    });
    return b;
  }
  // A new projection for the box {w, h, top, bottom} (top, bottom: the least room the furniture needs): padding
  // 18 % left and right, 25 % top, 28 % bottom; at least 20 x 12 degrees shown; centre rounded to 1 degree and
  // the scale to 10 % steps (rounded down, so the route stays inside the padding).
  function fitNew(pts, box) {
    var lc = Math.round(lonCentre(pts));
    var lats = pts.map(function (p) { return p[1]; });
    var mid = (Math.min.apply(null, lats) + Math.max.apply(null, lats)) / 2;
    var all = pts.concat([[lc - 10, mid], [lc + 10, mid], [lc, Math.max(-85, mid - 6)], [lc, Math.min(85, mid + 6)]]);
    var x0 = Math.round(box.w * 0.18), x1 = box.w - x0;
    var y0 = Math.max(box.top, Math.round(box.h * 0.25)), y1 = box.h - Math.max(box.bottom, Math.round(box.h * 0.28));
    if (y1 - y0 < 80) { y0 = Math.round(box.h * 0.2); y1 = Math.round(box.h * 0.75); }
    var proj = d3.geoEqualEarth().rotate([-lc, 0]).precision(0.3);
    proj.fitExtent([[x0, y0], [x1, y1]], { type: 'MultiPoint', coordinates: all });
    var s = Math.pow(1.1, Math.floor(Math.log(proj.scale()) / Math.log(1.1)));
    proj.scale(s).translate([0, 0]);
    var b = boundsOf(proj, all);
    proj.translate([Math.round((x0 + x1) / 2 - (b[0] + b[2]) / 2), Math.round((y0 + y1) / 2 - (b[1] + b[3]) / 2)]);
    return { proj: proj, lc: lc, scale: s, id: lc + ':' + s.toFixed(3) + ':' + proj.translate().join(',') };
  }
  function insideBox(proj, pts, box, inset) {
    var ix = box.w * inset, iy = box.h * inset;
    return pts.every(function (ll) {
      var p = proj(ll);
      return okP(p) && p[0] >= ix && p[0] <= box.w - ix && p[1] >= iy && p[1] <= box.h - iy;
    });
  }
  // The projection to draw with. key: target, home and size; while it stays the same, a fitted projection is kept as
  // long as the route still fits (all points inside the box inset by 10 %, the wanted scale within 0.6-1.5 of the
  // current one), so a route that changes a little does not make the map jump.
  function fit(nodes, box, key, prev) {
    var pts = fitPoints(nodes), want = fitNew(pts, box);
    if (prev && prev.key === key) {
      var ratio = want.scale / prev.scale;
      if (ratio >= 0.6 && ratio <= 1.5 && insideBox(prev.proj, pts, box, 0.1)) return prev;
    }
    want.key = key;
    return want;
  }
  // Grid step by the size of 10 degrees on screen: 10, 5 or 2 degrees; cell: pixels per step (for the crosses).
  function gridStep(proj, w, h) {
    var c = inv(proj, [w / 2, h / 2]) || [0, 0], a = proj([c[0], c[1]]), b = proj([c[0] + 10, c[1]]);
    var px10 = okP(a) && okP(b) ? dist(a, b) : 100;
    var step = px10 < 200 ? 10 : px10 < 500 ? 5 : 2;
    return { step: step, cell: px10 * step / 10 };
  }
  // Where the grid lines of `step` degrees cross one line of the map: along a row y (meridians: lon) or a column x
  // (parallels: lat). [{v: degrees, at: px}]
  function crossings(proj, fixed, alongX, len, step) {
    var out = [], prev = null, prevAt = 0;
    for (var t = 0; t <= len + 2; t += 2) {
      var tt = Math.min(t, len), q = inv(proj, alongX ? [tt, fixed] : [fixed, tt]);
      var v = q ? (alongX ? q[0] : q[1]) : null;
      if (v != null && prev != null && Math.abs(v - prev) < 45) {
        var a = Math.floor(prev / step), b = Math.floor(v / step);
        if (a !== b) {
          var k = Math.max(a, b) * step;
          out.push({ v: k, at: prevAt + (tt - prevAt) * (k - prev) / (v - prev) });
        }
      }
      prev = v; prevAt = tt;
    }
    return out;
  }

  // ---------- screen geometry ----------
  // Great-circle arc between two [lon, lat] points as screen points; broken where it jumps across the map's edge.
  function arc(proj, a, b, steps, w) {
    var f = d3.geoInterpolate(a, b), parts = [[]];
    for (var i = 0; i <= steps; i++) {
      var p = proj(f(i / steps)), cur = parts[parts.length - 1];
      if (!okP(p)) continue;
      if (cur.length && Math.abs(p[0] - cur[cur.length - 1][0]) > w / 2) parts.push(cur = []);
      cur.push(p);
    }
    return parts.filter(function (x) { return x.length > 1; });
  }
  // The part of a polyline outside a circle (where a line meets a ring or a marker).
  function clip(pts, c, r) {
    function inside(p) { return dist(p, c) < r; }
    function cross(a, b) {
      var lo = 0, hi = 1;
      for (var k = 0; k < 24; k++) {
        var m = (lo + hi) / 2, p = [a[0] + (b[0] - a[0]) * m, a[1] + (b[1] - a[1]) * m];
        if (inside(p) === inside(a)) lo = m; else hi = m;
      }
      return [a[0] + (b[0] - a[0]) * lo, a[1] + (b[1] - a[1]) * lo];
    }
    var out = [];
    pts.forEach(function (p, i) {
      var prev = pts[i - 1];
      if (!inside(p)) { if (prev && inside(prev)) out.push(cross(prev, p)); out.push(p); }
      else if (prev && !inside(prev)) out.push(cross(prev, p));
    });
    return out;
  }
  function length(pts) { var s = 0; for (var i = 1; i < pts.length; i++) s += dist(pts[i - 1], pts[i]); return s; }
  // Point at a share of a polyline's length.
  function along(pts, share) {
    if (!pts.length) return null;
    var need = length(pts) * Math.max(0, Math.min(1, share));
    for (var i = 1; i < pts.length; i++) {
      var d = dist(pts[i - 1], pts[i]);
      if (need <= d && d > 0) { var f = need / d; return [pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * f, pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * f]; }
      need -= d;
    }
    return pts[pts.length - 1].slice();
  }

  // Which marker a group shows: own hops without a home rank after every place.
  function rank(n) { return n.kind === 'own' && n.noHome ? 4.5 : RANK[n.kind]; }
  // Groups: markers that touch (closer than rA + rB + 2 px, consecutive or not) share the point of the surest one.
  function group(list) {
    var parent = list.map(function (x, i) { return i; });
    function root(i) { while (parent[i] !== i) { parent[i] = parent[parent[i]]; i = parent[i]; } return i; }
    for (var i = 0; i < list.length; i++) {
      for (var j = i + 1; j < list.length; j++) {
        if (dist(list[i].p, list[j].p) < RAD[list[i].kind] + RAD[list[j].kind] + 2) parent[root(j)] = root(i);
      }
    }
    var by = {}, out = [];
    list.forEach(function (n, k) { var r = root(k); (by[r] = by[r] || []).push(n); });
    Object.keys(by).forEach(function (k) {
      var g = by[k], best = g[0];
      g.forEach(function (n) { if (rank(n) < rank(best) || (rank(n) === rank(best) && n.ttl < best.ttl)) best = n; });
      g.forEach(function (n) { n.p = best.p.slice(); n.ll = best.ll; n.straight = best.straight; n.mk = best.kind; });
      g.sort(function (a, b) { return a.ttl - b.ttl; });
      out.push({ p: best.p.slice(), mk: best.kind, nodes: g, ttls: g.map(function (n) { return n.ttl; }) });
    });
    out.sort(function (a, b) { return a.ttls[0] - b.ttls[0]; });
    return out;
  }
  // Direction of the line arriving at nodes[i] (from the last node at another point), for the stub after it.
  function lastDir(nodes, i) {
    for (var j = i - 1; j >= 0; j--) {
      if (nodes[j].p && !sameP(nodes[j].p, nodes[i].p)) {
        var dx = nodes[i].p[0] - nodes[j].p[0], dy = nodes[i].p[1] - nodes[j].p[1], d = Math.sqrt(dx * dx + dy * dy);
        return [dx / d, dy / d];
      }
    }
    return null;
  }
  function unsure(n) { return n.kind === 'unknown' || n.kind === 'impossible'; }

  // Screen points, groups, route lines, the pointer of an anchored destination and the packet path for a projection.
  // Unknown and impossible hops go at shares i/(k+1) of the free part of the arc between their neighbours (k in a
  // row), into the previous group when that part is shorter than 22 px per hop, or at the end of a dashed 28 px stub
  // after the last place when nothing known follows them. avoid: boxes {x, y, w, h} the edge pointer stops short of
  // (the map's readout, legend and scale).
  function place(nodes, proj, w, h, avoid) {
    nodes.forEach(function (n) { n.p = n.ll ? proj(n.ll) : null; if (n.p && !okP(n.p)) n.p = null; n.straight = false; });
    group(nodes.filter(function (n) { return n.p; }));
    var i = 0;
    while (i < nodes.length) {
      if (nodes[i].p) { i++; continue; }
      var j = i;
      while (j < nodes.length && !nodes[j].p) j++;
      var run = nodes.slice(i, j), k = run.length, prev = i > 0 ? nodes[i - 1] : null, next = j < nodes.length ? nodes[j] : null;
      if (prev && next) {
        var fold = sameP(prev.p, next.p), free = null;
        if (!fold) {
          var parts = prev.ll && next.ll && !prev.straight && !next.straight ? arc(proj, prev.ll, next.ll, 64, w) : [[prev.p, next.p]];
          free = parts.length === 1 ? clip(clip(parts[0], prev.p, GAP[prev.mk]), next.p, GAP[next.mk]) : [];
          fold = length(free) < 22 * k;
        }
        run.forEach(function (n, t) {
          n.p = fold ? prev.p.slice() : along(free, (t + 1) / (k + 1));
          n.ll = fold ? prev.ll : inv(proj, n.p);
          n.straight = fold ? prev.straight : !n.ll;
        });
      } else if (prev) {
        var dir = lastDir(nodes, i - 1) || [1, 0], r = GAP[prev.mk] + 28;
        var end = [Math.max(12, Math.min(w - 12, prev.p[0] + dir[0] * r)), Math.max(12, Math.min(h - 12, prev.p[1] + dir[1] * r))];
        run.forEach(function (n) { n.p = end.slice(); n.ll = null; n.straight = true; n.stub = true; });
      } else if (next) {
        run.forEach(function (n) { n.p = next.p.slice(); n.ll = next.ll; n.straight = next.straight; });
      }
      i = j;
    }
    var placed = nodes.filter(function (n) { return n.p; });
    var groups = group(placed);
    groups.forEach(function (g, gi) { g.nodes.forEach(function (n) { n.g = gi; }); });

    // Route lines between consecutive nodes: dashed where a neighbour's place is unknown, stopping at a country ring.
    var segs = [];
    for (var s = 1; s < placed.length; s++) {
      var a = placed[s - 1], b = placed[s];
      if (sameP(a.p, b.p)) continue;
      var pp = a.ll && b.ll && !a.straight && !b.straight ? arc(proj, a.ll, b.ll, 64, w) : [[a.p.slice(), b.p.slice()]];
      if (pp.length) { pp[0][0] = a.p.slice(); pp[pp.length - 1][pp[pp.length - 1].length - 1] = b.p.slice(); }
      pp = pp.map(function (q) {
        if (a.mk === 'country') q = clip(q, a.p, RING + 3);
        if (b.mk === 'country') q = clip(q, b.p, RING + 3);
        return q;
      }).filter(function (q) { return q.length > 1; });
      segs.push({ a: a, b: b, parts: pp, dashed: unsure(a) || unsure(b) });
    }
    // Route direction at every group (for the callouts: not along the line).
    groups.forEach(function (g) { g.tan = [0, 0]; });
    segs.forEach(function (sg) {
      sg.parts.forEach(function (q) {
        var d0 = [q[1][0] - q[0][0], q[1][1] - q[0][1]], n0 = q.length - 1, d1 = [q[n0][0] - q[n0 - 1][0], q[n0][1] - q[n0 - 1][1]];
        [[q[0], d0], [q[n0], d1]].forEach(function (e) {
          groups.forEach(function (g) {
            if (dist(g.p, e[0]) < RING + 4) { var l = Math.sqrt(e[1][0] * e[1][0] + e[1][1] * e[1][1]) || 1; g.tan[0] += e[1][0] / l; g.tan[1] += e[1][1] / l; }
          });
        });
      });
    });
    groups.forEach(function (g) {
      var l = Math.sqrt(g.tan[0] * g.tan[0] + g.tan[1] * g.tan[1]);
      g.tan = l > 0.01 ? [g.tan[0] / l, g.tan[1] / l] : null;
    });
    // Route samples every 6 px (callouts pay for covering the route).
    var samples = [];
    segs.forEach(function (sg) {
      sg.parts.forEach(function (q) {
        var L0 = length(q);
        for (var t = 0; t <= L0; t += 6) samples.push(along(q, L0 ? t / L0 : 0));
      });
    });
    // The anchored destination: a pointer along the great circle towards its database point.
    var edge = null, an = nodes.filter(function (n) { return n.kind === 'anchored' && n.p && n.ll && n.db; })[0];
    if (an) edge = edgeToward(proj, an.ll, an.db, w, h, 16, avoid);
    if (edge) edge.ttl = an.ttl;
    return { nodes: nodes, placed: placed, groups: groups, segs: segs, samples: samples, edge: edge, path: path(placed, segs) };
  }

  // Where the great circle from one point towards another leaves the map (inset px from the edge) or would run into
  // one of the avoid boxes (the arrow's 20 px box must stay clear of them). inside: the database point itself is on
  // the map, it gets its own marker there (dbp) instead of the edge arrow.
  function edgeToward(proj, from, to, w, h, inset, avoid) {
    var f = d3.geoInterpolate(from, to), last = proj(from), line = [last];
    function blocked(p) {
      return (avoid || []).some(function (b) { return p[0] + 10 > b.x && p[0] - 10 < b.x + b.w && p[1] + 10 > b.y && p[1] - 10 < b.y + b.h; });
    }
    for (var i = 1; i <= 400; i++) {
      var p = proj(f(i / 400));
      if (!okP(p) || p[0] < inset || p[1] < inset || p[0] > w - inset || p[1] > h - inset || Math.abs(p[0] - last[0]) > w / 2 || blocked(p)) {
        var ang = okP(p) && Math.abs(p[0] - last[0]) <= w / 2 ? Math.atan2(p[1] - last[1], p[0] - last[0]) : Math.atan2(last[1] - line[Math.max(0, line.length - 2)][1], last[0] - line[Math.max(0, line.length - 2)][0]);
        return { point: [Math.max(inset, Math.min(w - inset, last[0])), Math.max(inset, Math.min(h - inset, last[1]))], angle: ang, inside: false, line: line };
      }
      line.push(p);
      last = p;
    }
    return { point: last, dbp: last, angle: 0, inside: true, line: line };
  }

  // The packet's path: every placed node with its point and the line from the node before it, timed by clock()
  // with the hops' window minimums as they were for this layout.
  function path(placed, segs) {
    var out = placed.map(function (n) { return { ttl: n.ttl, p: n.p, poly: [n.p], node: n }; }), hop = {};
    segs.forEach(function (sg) {
      var k = placed.indexOf(sg.b), pts = [];
      sg.parts.forEach(function (q) { pts = pts.concat(q); });
      if (k >= 0 && pts.length > 1) out[k].poly = [sg.a.p].concat(pts, [sg.b.p]);
    });
    placed.forEach(function (n) { hop[n.ttl] = n.hop; });
    return clock(out, function (ttl) { var h = hop[ttl]; return h && h.full ? h.full.min : null; });
  }
  // The clock of a path (RTT in ms) from minOf(ttl), the window minimum of a hop (null when it has none): raw is that
  // measured value, c the clock: nodes without a value interpolated between known ones, made monotone so the packet
  // never goes back (c_i >= c_(i-1) + 0.02 c_dest). The map runs it again with fresh numbers before drawing a packet.
  function clock(pth, minOf) {
    var i, out = pth;
    out.forEach(function (e) { var v = e.ttl === 0 ? 0 : minOf(e.ttl); e.raw = fin(v) ? v : null; e.c = e.raw; });
    for (i = 0; i < out.length; i++) {
      if (out[i].c != null) continue;
      var a = i - 1, b = i + 1;
      while (b < out.length && out[b].c == null) b++;
      var ca = a >= 0 ? out[a].c : 0, cb = b < out.length ? out[b].c : null;
      out[i].c = cb == null ? ca : ca + (cb - ca) * (i - a) / (b - a);
    }
    var cd = out.length ? out[out.length - 1].c : 0;
    if (!(cd > 0)) cd = Math.max.apply(null, out.map(function (x) { return x.c || 0; }).concat([1]));
    for (i = 1; i < out.length; i++) out[i].c = Math.max(out[i].c, out[i - 1].c + 0.02 * cd);
    return out;
  }
  // Where the packet is at clock r (ms): point, the hop it passed last and the next one.
  function posAt(pth, r) {
    if (!pth.length) return null;
    if (r <= pth[0].c) return { p: pth[0].p, lastTtl: pth[0].ttl, nextTtl: pth.length > 1 ? pth[1].ttl : null };
    for (var i = 1; i < pth.length; i++) {
      if (r <= pth[i].c) {
        var span = pth[i].c - pth[i - 1].c, f = span > 0 ? (r - pth[i - 1].c) / span : 1;
        return { p: along(pth[i].poly, f) || pth[i].p, lastTtl: f >= 1 ? pth[i].ttl : pth[i - 1].ttl, nextTtl: pth[i].ttl };
      }
    }
    var e = pth[pth.length - 1];
    return { p: e.p, lastTtl: e.ttl, nextTtl: null };
  }

  window.MAPG = {
    RING: RING, kinds: kinds, fit: fit, gridStep: gridStep, crossings: crossings, place: place, clock: clock, posAt: posAt,
    dist: dist, along: along, norm: norm, inv: inv,
  };
})();
