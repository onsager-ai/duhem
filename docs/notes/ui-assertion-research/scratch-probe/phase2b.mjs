// THROWAWAY PROBE — Phase 2b: class-D cross-viewport sweep.
// Metamorphic: no definition of "good" needed, only consistency between
// where layout relations change and where the page *declares* it changes.
import { chromium } from 'playwright';
import fs from 'node:fs';
import { COLLECTOR, settle } from './collect.mjs';
import { checkPage } from './phase2a.mjs';

const WIDTHS = [320, 375, 414, 768, 1024, 1280, 1440, 1920];
const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TIMEOUT ' + l)), ms))]);

// declared breakpoints, read out of the archive's own stylesheets
const BREAKPOINTS = `(() => {
  const out = new Set();
  const walk = (rules) => {
    for (const r of rules) {
      if (r.type === 4 || r.constructor.name === 'CSSMediaRule') {
        for (const m of (r.conditionText || r.media?.mediaText || '').matchAll(/(min|max)-width\\s*:\\s*([0-9.]+)(px|em|rem)/g)) {
          let v = parseFloat(m[2]);
          if (m[3] !== 'px') v *= 16;
          out.add(m[1] === 'max' ? Math.round(v) + 1 : Math.round(v));
        }
      }
      if (r.cssRules) try { walk(r.cssRules); } catch (e) {}
    }
  };
  for (const s of document.styleSheets) { try { walk(s.cssRules); } catch (e) {} }
  return [...out].sort((a,b)=>a-b);
})()`;

async function main() {
  const SHARD = Number(process.env.SHARD ?? 0), NSHARD = Number(process.env.NSHARD ?? 1);
  const prov = JSON.parse(fs.readFileSync('archives/provenance.json', 'utf8')).filter(r => r.usable).filter((_, i) => i % NSHARD === SHARD);
  const browser = await chromium.launch({ headless: true });
  const out = [];
  for (const s of prov) {
    const file = process.cwd() + '/archives/' + s.id + '.mhtml';
    if (!fs.existsSync(file)) continue;
    const row = { id: s.id, genre: s.genre, widths: {}, declared: [] };
    for (const w of WIDTHS) {
      const ctx = await browser.newContext({ viewport: { width: w, height: 900 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' });
      await ctx.addInitScript(`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});try{performance.now=()=>0;}catch(e){}})()`);
      await ctx.route('**/*', r => r.request().url().startsWith('file:') ? r.continue() : r.abort());
      const page = await ctx.newPage();
      try {
        await T(page.goto('file://' + file, { waitUntil: 'load', timeout: 40000 }), 45000, 'goto');
        await T(page.evaluate(async () => {
          window.scrollTo(0, 0);
          for (const svg of document.querySelectorAll('svg')) { try { svg.setCurrentTime?.(0); svg.pauseAnimations?.(); } catch (e) {} }
          try { for (const a of document.getAnimations?.() ?? []) { try { a.pause(); a.currentTime = 0; } catch (e) {} } } catch (e) {}
          if (document.fonts) { try { await document.fonts.ready; } catch (e) {} }
          await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
        }), 25000, 'settle').catch(() => {});
        const facts = await T(page.evaluate(COLLECTOR), 40000, 'collect');
        if (!row.declared.length) row.declared = await page.evaluate(BREAKPOINTS).catch(() => []);
        const r = checkPage(facts);
        row.widths[w] = {
          hScroll: facts.meta.scrollW > facts.meta.vw + 1,
          scrollW: facts.meta.scrollW, vw: facts.meta.vw,
          overflowFails: r.fail.filter(f => f.check === 'C1.viewport').length,
          overlapFails: r.fail.filter(f => f.check === 'B1.overlap').length,
          protrudeFails: r.fail.filter(f => f.check === 'B2.protrude').length,
          incon: r.inconclusive.length,
          visEls: r.stats.visEls,
        };
      } catch (e) { row.widths[w] = { error: e.message.slice(0, 50) }; }
      await ctx.close().catch(() => {});
    }
    // change points in the verdict vector
    const sig = w => { const d = row.widths[w]; return d && !d.error ? `${d.hScroll}|${d.overflowFails > 0}|${d.overlapFails > 0}` : 'err'; };
    row.changePoints = [];
    for (let i = 1; i < WIDTHS.length; i++) {
      if (sig(WIDTHS[i]) !== sig(WIDTHS[i - 1]) && sig(WIDTHS[i]) !== 'err' && sig(WIDTHS[i - 1]) !== 'err') {
        row.changePoints.push({ from: WIDTHS[i - 1], to: WIDTHS[i], sigFrom: sig(WIDTHS[i - 1]), sigTo: sig(WIDTHS[i]) });
      }
    }
    // does a declared breakpoint fall inside the interval where behaviour changed?
    row.unexplained = row.changePoints.filter(cp => !row.declared.some(d => d > cp.from && d <= cp.to));
    row.hScrollWidths = WIDTHS.filter(w => row.widths[w]?.hScroll);
    out.push(row);
    console.log(s.id.padEnd(24), 'hScroll@', (row.hScrollWidths.join(',') || '-').padEnd(24),
      'chg', row.changePoints.length, 'unexplained', row.unexplained.length, 'declaredBPs', row.declared.length);
    fs.writeFileSync(`out/phase2b.shard${SHARD}.json`, JSON.stringify(out, null, 2));
  }
  await browser.close();
  console.log('\npages with hScroll at >=1 width:', out.filter(r => r.hScrollWidths.length).length, '/', out.length);
  console.log('pages with unexplained change points:', out.filter(r => r.unexplained.length).length);
}
main();
