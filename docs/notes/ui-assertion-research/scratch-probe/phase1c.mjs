// THROWAWAY PROBE — Phase 1b: settle protocol v2.
// v1 mixed two contradictory freeze strategies (CSS "jump to end" +
// JS "rewind to 0 and pause") and applied them AFTER first paint, so the
// outcome depended on which won the race. v2 applies a single strategy
// (remove animation entirely) before the first byte is parsed.
import { chromium } from 'playwright';
import fs from 'node:fs';
import { COLLECTOR } from './collect.mjs';

const N = Number(process.env.N || 5);
const ONLY = process.env.ONLY;
const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TIMEOUT ' + l)), ms))]);

const FREEZE_CSS = `*,*::before,*::after{
  animation:none!important;transition:none!important;
  caret-color:transparent!important;scroll-behavior:auto!important}`;

const INIT = `(() => {
  const inject = () => {
    if (document.getElementById('__freeze')) return;
    const s = document.createElement('style');
    s.id = '__freeze'; s.textContent = ${JSON.stringify(FREEZE_CSS)};
    (document.head || document.documentElement).appendChild(s);
  };
  if (document.documentElement) inject();
  document.addEventListener('DOMContentLoaded', inject, { once: true });
  new MutationObserver(inject).observe(document.documentElement || document, { childList: true, subtree: false });
  // determinism: no time- or entropy-dependent layout
  const t0 = 1700000000000;
  try { Date.now = () => t0; } catch (e) {}
  try { Math.random = () => 0.42; } catch (e) {}
  // rAF-driven JS animation computes its frame from the clock; pin the clock.
  try { performance.now = () => 0; } catch (e) {}
  try { const RAF = window.requestAnimationFrame;
        window.requestAnimationFrame = (cb) => RAF(() => cb(0)); } catch (e) {}
})()`;

async function runOnce(browser, file) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' });
  await ctx.addInitScript(INIT);
  await ctx.route('**/*', r => r.request().url().startsWith('file:') ? r.continue() : r.abort());
  const page = await ctx.newPage();
  try {
    await T(page.goto('file://' + file, { waitUntil: 'load', timeout: 40000 }), 45000, 'goto');
    await T(page.evaluate(async () => {
      window.scrollTo(0, 0);
      // CSS animation:none does NOT stop SVG SMIL (<animate>); pause it explicitly.
      for (const svg of document.querySelectorAll('svg')) {
        try { svg.setCurrentTime?.(0); svg.pauseAnimations?.(); } catch (e) {}
      }
      // WAAPI (element.animate()) survives CSS animation:none — pause explicitly.
      try { for (const a of document.getAnimations?.() ?? []) { try { a.pause(); a.currentTime = 0; } catch (e) {} } } catch (e) {}
      if (document.fonts) { try { await document.fonts.ready; } catch (e) {} }
      await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    }), 20000, 'settle').catch(() => {});
    // decode needs a cap: img.decode() never settles for a failed image
    await T(page.evaluate(() => Promise.all([...document.images].filter(i => !i.complete).map(i => i.decode().catch(() => {})))), 5000, 'decode').catch(() => {});
    await page.waitForTimeout(150);
    // late-created animations (IntersectionObserver etc.) need a second sweep
    await page.evaluate(() => { try { for (const a of document.getAnimations?.() ?? []) { try { a.pause(); a.currentTime = 0; } catch (e) {} } } catch (e) {} }).catch(() => {});
    return await T(page.evaluate(COLLECTOR), 40000, 'collect');
  } finally { await ctx.close().catch(() => {}); }
}

function diffFields(recs) {
  const drift = new Set(); const base = recs[0];
  for (const r of recs.slice(1)) {
    for (let i = 0; i < 4; i++) if (r.box[i] !== base.box[i]) drift.add('box.' + ['x', 'y', 'w', 'h'][i]);
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
  let prov = JSON.parse(fs.readFileSync('archives/provenance.json', 'utf8')).filter(r => r.usable);
  if (ONLY) prov = prov.filter(r => r.id === ONLY);
  prov = prov.filter((_, i) => i % NSHARD === SHARD);
  const browser = await chromium.launch({ headless: true });
  const report = []; const driftTally = {}; const examples = {};
  for (const site of prov) {
    const file = process.cwd() + '/archives/' + site.id + '.mhtml';
    if (!fs.existsSync(file)) continue;
    const runs = []; let err = null;
    for (let i = 0; i < N; i++) {
      try { runs.push(await runOnce(browser, file)); } catch (e) { err = e.message.slice(0, 60); break; }
    }
    if (runs.length < N) { report.push({ id: site.id, genre: site.genre, error: err }); console.log(site.id, 'ERR', err); continue; }
    const keySets = runs.map(r => new Set(r.els.map(e => e.p)));
    const common = [...keySets[0]].filter(p => keySets.every(s => s.has(p)));
    const union = new Set(runs.flatMap(r => r.els.map(e => e.p)));
    const byRun = runs.map(r => new Map(r.els.map(e => [e.p, e])));
    let identical = 0, geomIdentical = 0; const localDrift = {};
    for (const p of common) {
      const d = diffFields(byRun.map(m => m.get(p)));
      if (d.size === 0) identical++;
      if (![...d].some(k => k.startsWith('box.'))) geomIdentical++;
      for (const k of d) {
        localDrift[k] = (localDrift[k] || 0) + 1;
        driftTally[k] = (driftTally[k] || 0) + 1;
        if (!examples[k]) examples[k] = { page: site.id, path: p, values: byRun.map(m => k.startsWith('box.') ? m.get(p).box : (k.startsWith('style.') ? m.get(p).style[k.slice(6)] : m.get(p)[k])) };
      }
    }
    const row = {
      id: site.id, genre: site.genre, n: common.length, unionN: union.size,
      setStable: common.length === union.size,
      identicalPct: +(100 * identical / Math.max(common.length, 1)).toFixed(2),
      geomPct: +(100 * geomIdentical / Math.max(common.length, 1)).toFixed(2),
      topDrift: Object.entries(localDrift).sort((a, b) => b[1] - a[1]).slice(0, 5),
      scrollH: [...new Set(runs.map(r => r.meta.scrollH))],
    };
    report.push(row);
    fs.writeFileSync(`facts/v4_${site.id}.json`, JSON.stringify(runs[0]));
    console.log(site.id.padEnd(24), 'v4', String(row.identicalPct).padStart(6), '| geom', String(row.geomPct).padStart(6), '| setStable', row.setStable, '| scrollH', row.scrollH.join('/'));
    fs.writeFileSync(`out/phase1c.shard${SHARD}.json`, JSON.stringify({ N, report, driftTally, examples }, null, 2));
  }
  await browser.close();
}
main();
