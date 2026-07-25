// THROWAWAY PROBE — Phase 5c: target-distribution corpus (AI-generated pages).
// Same capture protocol and the same FROZEN detectors as the wild corpus, so
// the two failure distributions are directly comparable.
import { chromium } from 'playwright';
import fs from 'node:fs';
import { COLLECTOR } from './collect.mjs';
import { checkPage } from './phase2a.mjs';
import { TEXTRUNS, overlapCheck } from './phase2e.mjs';

const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TO ' + l)), ms))]);
const FREEZE = `(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;

// Discovered via domain-scoped web search. bolt.host and lovable.app host
// generator output exclusively; replit.app is mixed (Agent-generated and
// hand-written Repls) so its provenance is weaker — flagged in the writeup.
const TARGETS = [
  ['bolt', 'https://project-portfolio-re-a1xv.bolt.host/'],
  ['bolt', 'https://saas-bcnf.bolt.host/'],
  ['bolt', 'https://personal-portfolio-w-ok7x.bolt.host/'],
  ['bolt', 'https://supplyhub.bolt.host/'],
  ['bolt', 'https://pottery-e-commerce-p-9pe0.bolt.host/'],
  ['bolt', 'https://suggestomatic-red.bolt.host/'],
  ['lovable', 'https://connectyourpms.lovable.app/'],
  ['lovable', 'https://tide-crm.lovable.app/'],
  ['lovable', 'https://contxai.lovable.app/'],
  ['lovable', 'https://pauldee-content-hub.lovable.app/'],
  ['lovable', 'https://pawsitive-crm-builder.lovable.app/'],
  ['lovable', 'https://fresha-setup.lovable.app/'],
  ['lovable', 'https://dasuccess-ecommerce.lovable.app/'],
  ['lovable', 'https://travel-horizon-flow.lovable.app/'],
  ['replit', 'https://uptimedash.replit.app/'],
];

async function main() {
  fs.mkdirSync('gen-archives', { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const results = [];
  for (const [gen, url] of TARGETS) {
    const id = gen + '_' + new URL(url).hostname.split('.')[0].replace(/[^a-z0-9]/gi, '_');
    const rec = { id, generator: gen, url, ok: false };
    try {
      const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' });
      await ctx.addInitScript(FREEZE);
      await ctx.route('**/*', async route => { const rq = route.request();
        try { const r = await T(fetch(rq.url(), { method: rq.method(), headers: rq.headers(), redirect: 'follow' }), 20000, 'f');
          const b = Buffer.from(await r.arrayBuffer()); const h = {};
          for (const [k, v] of r.headers) if (!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k] = v;
          await route.fulfill({ status: r.status, headers: h, body: b }); } catch { try { await route.abort(); } catch {} } });
      const page = await ctx.newPage();
      const resp = await T(page.goto(url, { waitUntil: 'load', timeout: 55000 }), 60000, 'goto');
      rec.status = resp?.status();
      try { await T(page.evaluate(() => document.fonts?.ready), 8000, 'fonts'); } catch {}
      try { await T(page.evaluate(async () => { const s = window.innerHeight;
        for (let y=0;y<Math.min(document.body.scrollHeight,25000);y+=s){window.scrollTo(0,y);await new Promise(r=>setTimeout(r,110));}
        window.scrollTo(0,0); }), 30000, 'scroll'); } catch {}
      await page.waitForTimeout(1000);
      await page.evaluate(() => { for (const s of document.querySelectorAll('svg')) { try { s.setCurrentTime?.(0); s.pauseAnimations?.(); } catch (e) {} }
        try { for (const a of document.getAnimations?.() ?? []) { try { a.pause(); a.currentTime = 0; } catch (e) {} } } catch (e) {} });
      try { await T(page.evaluate(() => Promise.all([...document.images].filter(i=>!i.complete).map(i=>i.decode().catch(()=>{})))), 6000, 'decode'); } catch {}

      rec.renderedEls = await page.evaluate(() => document.querySelectorAll('*').length);
      const facts = await T(page.evaluate(COLLECTOR), 40000, 'collect');
      const runs = await T(page.evaluate(TEXTRUNS), 40000, 'runs');
      const stat = checkPage(facts);
      const ov = overlapCheck(runs);
      const cdp = await ctx.newCDPSession(page);
      const snap = await T(cdp.send('Page.captureSnapshot', { format: 'mhtml' }), 90000, 'cdp');
      fs.writeFileSync(`gen-archives/${id}.mhtml`, snap.data);
      rec.archiveKB = Math.round(snap.data.length / 1024);
      rec.usable = rec.renderedEls >= 150;
      rec.stats = stat.stats;
      rec.fail = stat.fail.reduce((a,f)=>{a[f.check]=(a[f.check]||0)+1;return a},{});
      rec.failTotal = stat.fail.length;
      rec.inconclusive = stat.inconclusive.length;
      rec.suppressed = stat.suppressed.length;
      rec.textrunFail = ov.fail.length;
      rec.textrunStacked = ov.fail.filter(f=>f.cover>=0.99).length;
      rec.textrunRuns = ov.runs;
      rec.ok = true;
      await ctx.close().catch(()=>{});
      console.log('OK', gen.padEnd(8), id.padEnd(34), `els=${rec.renderedEls} fail=${rec.failTotal} tr=${rec.textrunFail} fs=${rec.stats.fontSize.distinct}/n90=${rec.stats.fontSize.n90} sp=${rec.stats.spacing.distinct}/n90=${rec.stats.spacing.n90} col=${rec.stats.color.distinct}`);
    } catch (e) {
      rec.error = e.message.split('\n')[0].slice(0, 90);
      console.log('FAIL', gen, id, rec.error);
    }
    results.push(rec);
    fs.writeFileSync('out/phase5c-topup.json', JSON.stringify(results, null, 2));
    await new Promise(r => setTimeout(r, 800));
  }
  await browser.close();
  const ok = results.filter(r => r.ok && r.usable);
  console.log(`\nusable ${ok.length}/${TARGETS.length}`);
  for (const g of ['lovable','bolt','replit']) console.log(' ', g, ok.filter(r=>r.generator===g).length);
}
main();
