// THROWAWAY PROBE — Phase 1: Facts determinism.
// Loads each frozen archive N times, offline, under two settle protocols,
// and measures whether the Facts snapshot is byte-identical across runs.
import { chromium } from 'playwright';
import fs from 'node:fs';
import { COLLECTOR, settle } from './collect.mjs';

const N = Number(process.env.N || 5);
const VIEWPORT = { width: 1280, height: 800 };
const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TIMEOUT ' + l)), ms))]);

async function runOnce(browser, file, level) {
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC' });
  let external = 0;
  await ctx.route('**/*', r => { // hard offline: nothing but the archive itself
    const u = r.request().url();
    if (u.startsWith('file:')) return r.continue();
    external++; return r.abort();
  });
  const page = await ctx.newPage();
  try {
    await T(page.goto('file://' + file, { waitUntil: 'load', timeout: 40000 }), 45000, 'goto');
    await T(settle(page, level), 30000, 'settle').catch(() => {});
    const facts = await T(page.evaluate(COLLECTOR), 40000, 'collect');
    facts.meta.externalBlocked = external;
    return facts;
  } finally { await ctx.close().catch(() => {}); }
}

// field-level comparison of one element across runs
function diffFields(recs) {
  const drift = new Set();
  const base = recs[0];
  for (const r of recs.slice(1)) {
    if (JSON.stringify(r.box) !== JSON.stringify(base.box)) {
      for (let i = 0; i < 4; i++) if (r.box[i] !== base.box[i]) drift.add('box.' + ['x', 'y', 'w', 'h'][i]);
    }
    for (const k of Object.keys(base.style)) if (r.style[k] !== base.style[k]) drift.add('style.' + k);
    if (r.textLen !== base.textLen) drift.add('textLen');
    if (r.lineBoxes !== base.lineBoxes) drift.add('lineBoxes');
    if (r.vis !== base.vis) drift.add('vis');
    if (r.zi !== base.zi) drift.add('zi');
  }
  return drift;
}

async function main() {
  const SHARD = Number(process.env.SHARD ?? 0), NSHARD = Number(process.env.NSHARD ?? 1);
  const prov = JSON.parse(fs.readFileSync('archives/provenance.json', 'utf8')).filter(r => r.usable).filter((_, i) => i % NSHARD === SHARD);
  const browser = await chromium.launch({ headless: true });
  const report = [];
  const driftTally = {};
  for (const site of prov) {
    const file = process.cwd() + '/archives/' + site.id + '.mhtml';
    if (!fs.existsSync(file)) continue;
    const row = { id: site.id, genre: site.genre };
    for (const level of ['naive', 'settled']) {
      const runs = [];
      let err = null;
      for (let i = 0; i < N; i++) {
        try { runs.push(await runOnce(browser, file, level)); }
        catch (e) { err = e.message.slice(0, 60); break; }
      }
      if (runs.length < N) { row[level] = { error: err || 'short' }; continue; }
      // element-set stability
      const keySets = runs.map(r => new Set(r.els.map(e => e.p)));
      const common = [...keySets[0]].filter(p => keySets.every(s => s.has(p)));
      const union = new Set(runs.flatMap(r => r.els.map(e => e.p)));
      const setStable = common.length === union.size;
      const byRun = runs.map(r => new Map(r.els.map(e => [e.p, e])));
      let identical = 0, geomIdentical = 0;
      const localDrift = {};
      for (const p of common) {
        const recs = byRun.map(m => m.get(p));
        const d = diffFields(recs);
        if (d.size === 0) identical++;
        if (![...d].some(k => k.startsWith('box.'))) geomIdentical++;
        for (const k of d) {
          localDrift[k] = (localDrift[k] || 0) + 1;
          if (level === 'settled') driftTally[k] = (driftTally[k] || 0) + 1;
        }
      }
      row[level] = {
        n: common.length, unionN: union.size, setStable,
        identicalPct: +(100 * identical / Math.max(common.length, 1)).toFixed(2),
        geomPct: +(100 * geomIdentical / Math.max(common.length, 1)).toFixed(2),
        topDrift: Object.entries(localDrift).sort((a, b) => b[1] - a[1]).slice(0, 5),
        scrollH: runs.map(r => r.meta.scrollH),
        externalBlocked: runs[0].meta.externalBlocked,
      };
      if (level === 'settled') fs.writeFileSync(`facts/${site.id}.json`, JSON.stringify(runs[0]));
    }
    report.push(row);
    console.log(site.id.padEnd(26),
      'naive', String(row.naive?.identicalPct ?? row.naive?.error).padStart(7),
      '| settled', String(row.settled?.identicalPct ?? row.settled?.error).padStart(7),
      '| geom', String(row.settled?.geomPct ?? '-').padStart(7),
      '| setStable', row.settled?.setStable ?? '-');
    fs.writeFileSync(`out/phase1.shard${SHARD}.json`, JSON.stringify({ N, report, driftTally }, null, 2));
  }
  await browser.close();
  const ok = report.filter(r => r.settled && !r.settled.error);
  const mean = k => +(ok.reduce((a, r) => a + r.settled[k], 0) / Math.max(ok.length, 1)).toFixed(2);
  const meanNaive = +(report.filter(r => r.naive && !r.naive.error).reduce((a, r) => a + r.naive.identicalPct, 0) / Math.max(report.filter(r => r.naive && !r.naive.error).length, 1)).toFixed(2);
  console.log(`\nN=${N} pages=${ok.length}`);
  console.log('mean identical  naive:', meanNaive, ' settled:', mean('identicalPct'));
  console.log('mean geom settled:', mean('geomPct'));
  console.log('pages 100% settled:', ok.filter(r => r.settled.identicalPct === 100).length);
  console.log('pages 100% geom  :', ok.filter(r => r.settled.geomPct === 100).length);
  console.log('top drift fields :', Object.entries(driftTally).sort((a, b) => b[1] - a[1]).slice(0, 12));
}
main();
