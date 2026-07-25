// THROWAWAY PROBE — Phase 5a: recall under CSS fault injection.
//
// Detectors are FROZEN: checkPage() and overlapCheck() are imported
// unmodified from phase2a/phase2e. Nothing here tunes them.
//
// Ground truth is exact: we mutate one known element and ask whether the
// detector implicates that element. Severity is reported as the *achieved*
// geometric delta measured after injection, not the requested one — a
// requested 16px shift into a 20px gap produces no overlap at all, and
// grading against the request would understate recall.
import { chromium } from 'playwright';
import fs from 'node:fs';
import { COLLECTOR } from './collect.mjs';
import { checkPage } from './phase2a.mjs';
import { TEXTRUNS, overlapCheck } from './phase2e.mjs';
import { INJECT } from './inject.mjs';

const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TO ' + l)), ms))]);
const FREEZE = `(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;

const SEVERITIES = [0, 1, 4, 16, 48];   // 0 = baseline, no injection
const FAULTS = ['overlap', 'protrude', 'viewport'];


async function run(browser, file, fault, sev, only) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' });
  await ctx.addInitScript(FREEZE);
  await ctx.route('**/*', r => r.request().url().startsWith('file:') ? r.continue() : r.abort());
  const page = await ctx.newPage();
  try {
    await T(page.goto('file://' + file, { waitUntil: 'load', timeout: 40000 }), 45000, 'goto');
    await page.evaluate(() => { for (const s of document.querySelectorAll('svg')) { try { s.setCurrentTime?.(0); s.pauseAnimations?.(); } catch (e) {} }
      try { for (const a of document.getAnimations?.() ?? []) { try { a.pause(); a.currentTime = 0; } catch (e) {} } } catch (e) {} });
    await page.waitForTimeout(300);
    const inj = await T(page.evaluate(`${INJECT}(${JSON.stringify({ fault, sev, only: only || null })})`), 30000, 'inject');
    if (inj.skipped) return { skipped: inj.skipped };
    await page.waitForTimeout(120);
    const facts = await T(page.evaluate(COLLECTOR), 40000, 'collect');
    const runs = await T(page.evaluate(TEXTRUNS), 40000, 'runs');
    const stat = checkPage(facts);           // frozen detector
    const ov = overlapCheck(runs);           // frozen detector
    return { inj, stat, ov };
  } finally { await ctx.close().catch(() => {}); }
}

// did the frozen detector implicate the injected element?
function detected(fault, target, stat, ov) {
  if (fault === 'overlap') {
    const hit = ov.fail.find(f => f.a === target || f.b === target);
    return { hit: !!hit, localised: !!hit, n: ov.fail.length,
             gated: !!(ov.suppressed.find(f => f.a === target || f.b === target)
                    || ov.incon.find(f => f.a === target || f.b === target)) };
  }
  const key = fault === 'protrude' ? 'B2.protrude' : 'C1.viewport';
  const hits = stat.fail.filter(f => f.check === key);
  const hit = hits.find(f => f.a === target);
  return { hit: !!hit, localised: !!hit, n: hits.length,
           gated: !!(stat.suppressed.find(f => f.check === key && f.a === target)
                  || stat.inconclusive.find(f => f.check === key && f.a === target)) };
}

async function main() {
  const SHARD = Number(process.env.SHARD ?? 0), NSHARD = Number(process.env.NSHARD ?? 1);
  const prov = JSON.parse(fs.readFileSync('archives/provenance.json', 'utf8')).filter(r => r.usable).filter((_, i) => i % NSHARD === SHARD);
  const browser = await chromium.launch({ headless: true });
  const out = [];
  for (const s of prov) {
    const file = process.cwd() + '/archives/' + s.id + '.mhtml';
    if (!fs.existsSync(file)) continue;
    for (const fault of FAULTS) {
      let baselineFP = null;
      // pin one element per (page, fault) by probing at max severity, so the
      // whole ladder is measured on a constant element
      let pinned = null, pinNote = null;
      try {
        const probe = await run(browser, file, fault, SEVERITIES[SEVERITIES.length - 1], null);
        if (probe.skipped) pinNote = probe.skipped; else pinned = probe.inj.target;
      } catch (e) { pinNote = 'probe-error: ' + e.message.slice(0, 40); }
      if (!pinned) {
        out.push({ page: s.id, genre: s.genre, fault, skipped: pinNote || 'no-target' });
        console.log(s.id.padEnd(22), fault.padEnd(9), 'SKIP', pinNote);
        fs.writeFileSync(`out/phase5a.shard${SHARD}.json`, JSON.stringify(out, null, 2));
        continue;
      }
      for (const sev of SEVERITIES) {
        let rec = { page: s.id, genre: s.genre, fault, sev };
        try {
          const r = await run(browser, file, fault, sev, pinned);
          if (r.skipped) { rec.skipped = r.skipped; }
          else {
            const d = detected(fault, r.inj.target, r.stat, r.ov);
            const totalFP = r.stat.fail.length + r.ov.fail.length;
            if (sev === 0) baselineFP = totalFP;
            rec = { ...rec, target: r.inj.target, achieved: r.inj.achieved,
                    detected: d.hit, localised: d.localised, gated: d.gated,
                    classFindings: d.n, totalFindings: totalFP,
                    baselineFindings: baselineFP,
                    fpDelta: baselineFP == null ? null : totalFP - baselineFP };
          }
        } catch (e) { rec.error = e.message.slice(0, 200); if (!process.env.QUIET) console.log("  ERRDETAIL", fault, sev, e.message.split("\n")[0].slice(0,160)); }
        out.push(rec);
        fs.writeFileSync(`out/phase5a.shard${SHARD}.json`, JSON.stringify(out, null, 2));
      }
      const row = out.filter(r => r.page === s.id && r.fault === fault);
      console.log(s.id.padEnd(22), fault.padEnd(9),
        row.map(r => r.skipped ? 'skip' : r.error ? 'ERR' : `${r.sev}:${r.achieved ?? '-'}px${r.detected ? '✓' : r.gated ? 'g' : '✗'}`).join(' '));
    }
  }
  await browser.close();
}
main();
