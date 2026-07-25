// THROWAWAY PROBE — Phase 5b: pricing the responsive sweep.
//  B1 capture wall-time + archive size per width
//  B2 (computed in the writeup from B1)
//  B3 the width RANGE over which each known defect is observable -> the
//     minimum sampling density that would still catch it
//  B4 is archive infidelity mechanically detectable?
import { chromium } from 'playwright';
import fs from 'node:fs';

const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TO ' + l)), ms))]);
const FREEZE = `(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;

// observable horizontal scroll: attempt the scroll, don't trust scrollWidth
const SCROLLABLE = `(() => { const before=window.scrollX; window.scrollTo(9999,0);
  const after=window.scrollX; window.scrollTo(before,0);
  return { maxX: after, sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth }; })()`;

function liveCtx(browser, width) {
  return browser.newContext({ viewport: { width, height: 900 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' })
    .then(async ctx => {
      await ctx.addInitScript(FREEZE);
      await ctx.route('**/*', async route => { const rq = route.request();
        try { const r = await T(fetch(rq.url(), { method: rq.method(), headers: rq.headers(), redirect: 'follow' }), 20000, 'f');
          const b = Buffer.from(await r.arrayBuffer()); const h = {};
          for (const [k, v] of r.headers) if (!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k] = v;
          await route.fulfill({ status: r.status, headers: h, body: b }); } catch { try { await route.abort(); } catch {} } });
      return ctx;
    });
}
function archiveCtx(browser, width) {
  return browser.newContext({ viewport: { width, height: 900 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' })
    .then(async ctx => { await ctx.addInitScript(FREEZE);
      await ctx.route('**/*', r => r.request().url().startsWith('file:') ? r.continue() : r.abort()); return ctx; });
}

async function scrollableAt(browser, url, width, isLive, file) {
  const ctx = isLive ? await liveCtx(browser, width) : await archiveCtx(browser, width);
  const page = await ctx.newPage();
  try {
    await T(page.goto(isLive ? url : 'file://' + file, { waitUntil: 'load', timeout: 45000 }), 50000, 'goto');
    await page.waitForTimeout(isLive ? 1400 : 400);
    return await T(page.evaluate(SCROLLABLE), 20000, 'm');
  } finally { await ctx.close().catch(() => {}); }
}

async function main() {
  const prov = JSON.parse(fs.readFileSync('archives/provenance.json', 'utf8')).filter(r => r.usable);
  const browser = await chromium.launch({ headless: true });
  const out = { b1: [], b3: [], b4: [] };

  // ---- B1: capture cost per width -----------------------------------------
  const B1_PAGES = ['news.ycombinator.com', 'react.dev', 'stripe.com', 'theguardian.com'];
  const B1_WIDTHS = [320, 768, 1280];
  for (const id of B1_PAGES) {
    const site = prov.find(p => p.id === id); if (!site) continue;
    for (const w of B1_WIDTHS) {
      const t0 = Date.now();
      let bytes = null, err = null;
      try {
        const ctx = await liveCtx(browser, w);
        const page = await ctx.newPage();
        await T(page.goto(site.url, { waitUntil: 'load', timeout: 50000 }), 55000, 'goto');
        await page.waitForTimeout(1200);
        await T(page.evaluate(async () => { const s=window.innerHeight;
          for (let y=0;y<Math.min(document.body.scrollHeight,20000);y+=s){window.scrollTo(0,y);await new Promise(r=>setTimeout(r,80));}
          window.scrollTo(0,0); }), 30000, 'scroll').catch(()=>{});
        const cdp = await ctx.newCDPSession(page);
        const snap = await T(cdp.send('Page.captureSnapshot', { format: 'mhtml' }), 90000, 'cdp');
        bytes = snap.data.length;
        await ctx.close().catch(()=>{});
      } catch (e) { err = e.message.slice(0, 50); }
      const rec = { id, width: w, ms: Date.now() - t0, kb: bytes ? Math.round(bytes/1024) : null, err };
      out.b1.push(rec);
      console.log('B1', id.padEnd(22), w, rec.ms + 'ms', (rec.kb ?? '-') + 'KB', err || '');
      fs.writeFileSync('out/phase5b.json', JSON.stringify(out, null, 2));
    }
  }

  // ---- B3: observable width range of each known defect --------------------
  // bisect the boundary where the page stops being horizontally scrollable
  const DEFECTS = [
    { id: 'news.ycombinator.com', known: 768 },
    { id: 'allbirds.com', known: 320 },
    { id: 'app.diagrams.net', known: 1280 },
  ];
  for (const d of DEFECTS) {
    const site = prov.find(p => p.id === d.id); if (!site) continue;
    const probe = async (w) => { try { const m = await scrollableAt(browser, site.url, w, true); return m.maxX > 0; } catch { return null; } };
    const atKnown = await probe(d.known);
    // walk outward on a coarse grid to bracket the range, then bisect each edge
    const GRID = [320, 340, 360, 375, 414, 480, 560, 640, 700, 768, 840, 900, 1024, 1152, 1280, 1360, 1440, 1600, 1920];
    const marks = {};
    for (const w of GRID) marks[w] = await probe(w);
    const trueW = GRID.filter(w => marks[w]);
    const rec = { id: d.id, known: d.known, scrollableAtKnown: atKnown, grid: marks,
                  observableWidths: trueW,
                  span: trueW.length ? [Math.min(...trueW), Math.max(...trueW)] : null };
    out.b3.push(rec);
    console.log('B3', d.id.padEnd(22), 'observable at:', trueW.join(',') || 'none');
    fs.writeFileSync('out/phase5b.json', JSON.stringify(out, null, 2));
  }

  // ---- B4: is archive infidelity mechanically detectable? -----------------
  const B4_PAGES = ['stripe.com', 'news.ycombinator.com', 'react.dev', 'allbirds.com', 'docs.python.org', 'bbc.com'];
  for (const id of B4_PAGES) {
    const site = prov.find(p => p.id === id); if (!site) continue;
    const file = process.cwd() + '/archives/' + id + '.mhtml';
    for (const w of [320, 768, 1280]) {
      let a = null, l = null;
      try { a = await scrollableAt(browser, site.url, w, false, file); } catch (e) { a = { err: e.message.slice(0,40) }; }
      try { l = await scrollableAt(browser, site.url, w, true); } catch (e) { l = { err: e.message.slice(0,40) }; }
      const agree = a && l && !a.err && !l.err ? ((a.maxX > 0) === (l.maxX > 0)) : null;
      const rec = { id, width: w, atCapture: w === 1280,
                    archive: a, live: l, verdictAgrees: agree,
                    // candidate mechanical infidelity signal, computable from the pair
                    swRatio: a && l && l.sw ? +(a.sw / l.sw).toFixed(3) : null };
      out.b4.push(rec);
      console.log('B4', id.padEnd(22), w, 'archive hScroll', a?.maxX > 0, '| live', l?.maxX > 0, '| agree', agree, '| swRatio', rec.swRatio);
      fs.writeFileSync('out/phase5b.json', JSON.stringify(out, null, 2));
    }
  }
  await browser.close();
}
main();
