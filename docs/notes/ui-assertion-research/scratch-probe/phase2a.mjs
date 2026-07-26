// THROWAWAY PROBE — Phase 2a: failure distribution, static classes.
// Pure function over serialized Facts. No browser, no network.
// Four hardcoded checks, no DSL, no abstraction.
import fs from 'node:fs';

const isAnc = (a, b) => b.startsWith(a + '/');           // a is ancestor of b
const rel = (a, b) => isAnc(a, b) || isAnc(b, a);
const alpha = c => { const m = /rgba?\(([^)]+)\)/.exec(c || ''); if (!m) return 1; const p = m[1].split(',').map(s => parseFloat(s)); return p.length > 3 ? p[3] : 1; };
// Occlusion verdict for the element painting on top:
//  'opaque'  -> it provably hides what is underneath  (determinable => suppress)
//  'unknown' -> gradient/image background, no single resolvable colour (=> inconclusive)
//  'see-through' -> underneath shows through (=> the collision is observable)
const occlusion = (e) => {
  if (e.style.backgroundImage && e.style.backgroundImage !== 'none') return 'unknown';
  const a = alpha(e.style.backgroundColor);
  if (a === 1) return 'opaque';
  if (a === 0) return 'see-through';
  return 'unknown';
};
const area = b => b[2] * b[3];
const inter = (a, b) => {
  const x = Math.max(0, Math.min(a[0] + a[2], b[0] + b[2]) - Math.max(a[0], b[0]));
  const y = Math.max(0, Math.min(a[1] + a[3], b[1] + b[3]) - Math.max(a[1], b[1]));
  return [x, y, x * y];
};

export function checkPage(facts) {
  const els = facts.els;
  const byPath = new Map(els.map(e => [e.p, e]));
  const ancestorsOf = (p) => { const out = []; let i = p.lastIndexOf('/'); while (i > 0) { const q = p.slice(0, i); const e = byPath.get(q); if (e) out.push(e); i = q.lastIndexOf('/'); } return out; };

  // Observability gate, computed rather than inferred.
  // An ancestor with overflow!=visible only *establishes* a clip rect; whether
  // anything is actually clipped is a geometric question. Treating the property
  // itself as "clipped" marks ~everything inconclusive (measured: 89%).
  const clipRectOf = (e) => {
    let r = [-Infinity, -Infinity, Infinity, Infinity]; // x1,y1,x2,y2
    for (const a of ancestorsOf(e.p)) {
      if (/visible/.test(a.style.overflowX) && /visible/.test(a.style.overflowY)) continue;
      r = [Math.max(r[0], a.box[0]), Math.max(r[1], a.box[1]),
           Math.min(r[2], a.box[0] + a.box[2]), Math.min(r[3], a.box[1] + a.box[3])];
    }
    return r;
  };
  // fraction of a box that survives its clip rect
  const visibleFrac = (box, r) => {
    const w = Math.max(0, Math.min(box[0] + box[2], r[2]) - Math.max(box[0], r[0]));
    const h = Math.max(0, Math.min(box[1] + box[3], r[3]) - Math.max(box[1], r[1]));
    const a = box[2] * box[3];
    return a > 0 ? (w * h) / a : 1;
  };
  const clipped = (e) => visibleFrac(e.box, clipRectOf(e)) < 0.99;
  // is a specific region actually rendered for this element?
  const regionVisible = (e, rx, ry, rw, rh) => {
    const r = clipRectOf(e);
    const w = Math.max(0, Math.min(rx + rw, r[2]) - Math.max(rx, r[0]));
    const h = Math.max(0, Math.min(ry + rh, r[3]) - Math.max(ry, r[1]));
    return w > 2 && h > 2;
  };
  const vw = facts.meta.vw;
  const res = { fail: [], inconclusive: [], suppressed: [], stats: {} };

  // ---- B1: text-bearing overlap -------------------------------------------
  const text = els.filter(e => e.vis && e.textLen > 0 && e.box[2] > 1 && e.box[3] > 1 && area(e.box) > 0);
  const order = new Map(els.map((e, i) => [e.p, i]));
  let pairsTested = 0;
  // bucket by vertical band to keep it O(n * k)
  const BAND = 200;
  const bands = new Map();
  for (const e of text) {
    const b0 = Math.floor(e.box[1] / BAND), b1 = Math.floor((e.box[1] + e.box[3]) / BAND);
    for (let b = b0; b <= b1; b++) { if (!bands.has(b)) bands.set(b, []); bands.get(b).push(e); }
  }
  const seen = new Set();
  for (const list of bands.values()) {
    for (let i = 0; i < list.length; i++) for (let j = i + 1; j < list.length; j++) {
      const a = list[i], b = list[j];
      if (rel(a.p, b.p)) continue;
      const k = a.p < b.p ? a.p + '|' + b.p : b.p + '|' + a.p;
      if (seen.has(k)) continue; seen.add(k);
      pairsTested++;
      const [ix, iy, ia] = inter(a.box, b.box);
      if (ix <= 2 || iy <= 2) continue;
      const smaller = Math.min(area(a.box), area(b.box));
      if (ia / smaller < 0.25) continue;
      // observability gate
      const top = (parseInt(a.zi) || 0) !== (parseInt(b.zi) || 0)
        ? ((parseInt(a.zi) || 0) > (parseInt(b.zi) || 0) ? a : b)
        : (order.get(a.p) > order.get(b.p) ? a : b);
      const bottom = top === a ? b : a;
      const rec = { check: 'B1.overlap', a: a.p, b: b.p, ix, iy, cover: +(ia / smaller).toFixed(2) };
      // the collision region itself must actually be painted for both parties
      const ox = Math.max(a.box[0], b.box[0]), oy = Math.max(a.box[1], b.box[1]);
      if (!regionVisible(a, ox, oy, ix, iy) || !regionVisible(b, ox, oy, ix, iy)) {
        res.suppressed.push({ ...rec, why: 'collision-region-clipped' }); continue;
      }
      const occ = occlusion(top);
      if (occ === 'opaque') { res.suppressed.push({ ...rec, why: 'occluded-by-opaque-top' }); continue; }
      if (occ === 'unknown') { res.inconclusive.push({ ...rec, why: 'occluder-bg-unresolvable' }); continue; }
      if (top.style.position !== 'static' && bottom.style.position !== 'static') { res.inconclusive.push({ ...rec, why: 'both-positioned-overlay' }); continue; }
      res.fail.push(rec);
    }
  }
  res.stats.pairsTested = pairsTested;

  // ---- B2/C: protrusion and viewport containment --------------------------
  let protr = 0;
  for (const e of els) {
    if (!e.vis || e.box[2] <= 0) continue;
    const right = e.box[0] + e.box[2], left = e.box[0];
    // C: horizontal viewport escape
    if (right > vw + 1 || left < -1) {
      const rec = { check: 'C1.viewport', a: e.p, right: +right.toFixed(1), left: +left.toFixed(1), vw };
      if (clipped(e)) res.suppressed.push({ ...rec, why: 'clipped-ancestor' });
      else if (e.style.position === 'fixed' || e.style.position === 'absolute') res.inconclusive.push({ ...rec, why: 'positioned-offscreen' });
      else res.fail.push(rec);
    }
    // B: child protrudes past a non-clipping parent
    const par = byPath.get(e.p.slice(0, e.p.lastIndexOf('/')));
    if (par && par.vis && e.style.position === 'static' && par.box[2] > 0) {
      const over = (e.box[0] + e.box[2]) - (par.box[0] + par.box[2]);
      if (over > 1 && /visible/.test(par.style.overflowX)) {
        const rec = { check: 'B2.protrude', a: e.p, parent: par.p, overBy: +over.toFixed(1) };
        // protruding past a parent is only observable if the protruding strip is painted
        if (!regionVisible(e, par.box[0] + par.box[2], e.box[1], over, e.box[3])) {
          res.suppressed.push({ ...rec, why: 'protrusion-clipped-higher-up' });
        } else { protr++; res.fail.push(rec); }
      }
    }
  }
  res.stats.protrusions = protr;

  // ---- A/C: token self-consistency (internal regularity) -------------------
  const usage = (fn, filter = () => true) => {
    const m = new Map();
    for (const e of els) { if (!e.vis || !filter(e)) continue; const v = fn(e); if (v == null) continue; m.set(v, (m.get(v) || 0) + 1); }
    return m;
  };
  const nCover = (m, frac) => {
    const tot = [...m.values()].reduce((a, b) => a + b, 0);
    const sorted = [...m.values()].sort((a, b) => b - a);
    let acc = 0, n = 0;
    for (const v of sorted) { acc += v; n++; if (acc / tot >= frac) break; }
    return { distinct: m.size, n90: n, usages: tot };
  };
  const px = v => v && v.endsWith('px') ? Math.round(parseFloat(v) * 100) / 100 : null;
  res.stats.fontSize = nCover(usage(e => px(e.style.fontSize), e => e.textLen > 0), 0.9);
  res.stats.color = nCover(usage(e => e.style.color, e => e.textLen > 0), 0.9);
  const sp = new Map();
  for (const e of els) {
    if (!e.vis) continue;
    for (const k of ['paddingTop', 'paddingLeft', 'paddingBottom', 'paddingRight', 'marginTop', 'marginBottom', 'gap']) {
      const v = px(e.style[k]); if (v && v > 0) sp.set(v, (sp.get(v) || 0) + 1);
    }
  }
  res.stats.spacing = nCover(sp, 0.9);
  res.stats.visEls = els.filter(e => e.vis).length;
  return res;
}

if (process.argv[1] && process.argv[1].endsWith('phase2a.mjs')) {
  const prov = JSON.parse(fs.readFileSync('archives/provenance.json', 'utf8')).filter(r => r.usable);
  const all = [];
  for (const s of prov) {
    let f = `facts/v4_${s.id}.json`;
    if (!fs.existsSync(f)) f = `facts/v2_${s.id}.json`;
    if (!fs.existsSync(f)) f = `facts/${s.id}.json`;
    if (!fs.existsSync(f)) { console.log('skip (no facts)', s.id); continue; }
    const facts = JSON.parse(fs.readFileSync(f, 'utf8'));
    const r = checkPage(facts);
    all.push({ id: s.id, genre: s.genre, ...r });
    console.log(s.id.padEnd(24), 'fail', String(r.fail.length).padStart(4),
      'incon', String(r.inconclusive.length).padStart(4),
      '| fs', String(r.stats.fontSize.distinct).padStart(3), '/n90', String(r.stats.fontSize.n90).padStart(2),
      '| sp', String(r.stats.spacing.distinct).padStart(3), '/n90', String(r.stats.spacing.n90).padStart(2),
      '| col', String(r.stats.color.distinct).padStart(3));
  }
  fs.writeFileSync('out/phase2a.json', JSON.stringify(all, null, 2));
  const tally = {};
  for (const p of all) { for (const f of p.fail) tally[f.check] = (tally[f.check] || 0) + 1; }
  const itally = {}, stally = {};
  for (const p of all) { for (const f of p.inconclusive) itally[f.why] = (itally[f.why] || 0) + 1; }
  for (const p of all) { for (const f of p.suppressed) stally[f.why] = (stally[f.why] || 0) + 1; }
  console.log('SUPPRESSED (determinable non-violation):', stally);
  console.log('\nFAIL by check:', tally);
  console.log('INCONCLUSIVE by cause:', itally);
  const tf = Object.values(tally).reduce((a, b) => a + b, 0), ti = Object.values(itally).reduce((a, b) => a + b, 0);
  console.log('total fail', tf, 'total inconclusive', ti, 'incon rate', +(100 * ti / Math.max(tf + ti, 1)).toFixed(1) + '%');
}
