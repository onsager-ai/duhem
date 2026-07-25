// THROWAWAY PROBE — Phase 2c: build visual evidence for manual adjudication.
// Renders each sampled finding with the implicated boxes outlined, crops the
// region, and lays the crops out as numbered contact sheets so each finding
// can be judged by eye against the actual pixels.
import { chromium } from 'playwright';
import fs from 'node:fs';

const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TIMEOUT ' + l)), ms))]);
const FREEZE = `*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}`;

function sample(all, n, pred) {
  const pool = [];
  for (const p of all) for (const f of (pred(p) || [])) pool.push({ page: p.id, genre: p.genre, ...f });
  // deterministic stratified pick: spread across pages
  const byPage = new Map();
  for (const c of pool) { if (!byPage.has(c.page)) byPage.set(c.page, []); byPage.get(c.page).push(c); }
  const out = []; let i = 0;
  while (out.length < n) {
    let added = false;
    for (const [, list] of byPage) { if (list[i]) { out.push(list[i]); added = true; if (out.length >= n) break; } }
    if (!added) break; i++;
  }
  return out;
}

async function main() {
  const results = JSON.parse(fs.readFileSync('out/phase2a.json', 'utf8'));
  const cases = [
    ...sample(results, 12, p => p.fail.filter(f => f.check === 'B1.overlap')).map(c => ({ ...c, cls: 'B1' })),
    ...sample(results, 10, p => p.fail.filter(f => f.check === 'B2.protrude')).map(c => ({ ...c, cls: 'B2' })),
    ...sample(results, 8, p => p.fail.filter(f => f.check === 'C1.viewport')).map(c => ({ ...c, cls: 'C1' })),
  ];
  fs.mkdirSync('out/crops', { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const index = [];
  let n = 0;
  const byPage = new Map();
  for (const c of cases) { if (!byPage.has(c.page)) byPage.set(c.page, []); byPage.get(c.page).push(c); }

  for (const [pageId, list] of byPage) {
    const file = process.cwd() + '/archives/' + pageId + '.mhtml';
    if (!fs.existsSync(file)) continue;
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, locale: 'en-US', timezoneId: 'UTC', reducedMotion: 'reduce' });
    await ctx.addInitScript(`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent=${JSON.stringify(FREEZE)};(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`);
    await ctx.route('**/*', r => r.request().url().startsWith('file:') ? r.continue() : r.abort());
    const page = await ctx.newPage();
    try {
      await T(page.goto('file://' + file, { waitUntil: 'load', timeout: 40000 }), 45000, 'goto');
      await page.evaluate(() => { for (const s of document.querySelectorAll('svg')) { try { s.setCurrentTime?.(0); s.pauseAnimations?.(); } catch (e) {} } });
      await page.waitForTimeout(400);
      for (const c of list) {
        n++;
        const id = String(n).padStart(2, '0');
        const targets = [c.a, c.b, c.parent].filter(Boolean);
        const box = await page.evaluate(({ targets }) => {
          const find = (p) => {
            const parts = p.split('/').slice(1);
            let el = document.documentElement;
            for (const part of parts) {
              const [tag, idx] = part.split(':');
              const kids = el.children;
              el = kids[Number(idx) - 1];
              if (!el || el.tagName.toLowerCase() !== tag) return null;
            }
            return el;
          };
          const colors = ['#ff0000', '#0066ff', '#00aa00'];
          let u = null;
          targets.forEach((p, i) => {
            const el = find(p); if (!el) return;
            el.style.setProperty('outline', '3px solid ' + colors[i], 'important');
            el.style.setProperty('outline-offset', '0px', 'important');
            const b = el.getBoundingClientRect();
            const r = { x: b.x + scrollX, y: b.y + scrollY, w: b.width, h: b.height };
            u = u ? { x: Math.min(u.x, r.x), y: Math.min(u.y, r.y), x2: Math.max(u.x + u.w, r.x + r.w), y2: Math.max(u.y + u.h, r.y + r.h), get w() { return this.x2 - this.x; }, get h() { return this.y2 - this.y; } } : r;
          });
          if (!u) return null;
          return { x: Math.max(0, u.x - 40), y: Math.max(0, u.y - 40), w: Math.min(1280, (u.w ?? 0) + 80), h: Math.min(900, (u.h ?? 0) + 80) };
        }, { targets });
        if (!box || box.w < 4 || box.h < 4) { console.log('skip', id, c.page, c.cls); continue; }
        const f = `out/crops/${id}.png`;
        try {
          await page.screenshot({ path: f, fullPage: true, clip: { x: box.x, y: box.y, width: Math.max(60, box.w), height: Math.max(40, Math.min(box.h, 700)) } });
          index.push({ id, file: f, ...c });
        } catch (e) { console.log('shot fail', id, e.message.slice(0, 40)); }
        // clear outlines
        await page.evaluate(() => document.querySelectorAll('[style*="outline"]').forEach(e => e.style.removeProperty('outline')));
      }
    } catch (e) { console.log('page fail', pageId, e.message.slice(0, 50)); }
    await ctx.close().catch(() => {});
  }
  fs.writeFileSync('out/adjudication-index.json', JSON.stringify(index, null, 2));
  console.log('crops:', index.length);

  // contact sheets, 6 panels each
  const per = 6;
  for (let s = 0; s * per < index.length; s++) {
    const slice = index.slice(s * per, s * per + per);
    const html = `<html><body style="margin:0;background:#222;font:12px monospace;color:#fff">
    <div style="display:grid;grid-template-columns:1fr 1fr;gap:6px;padding:6px">
    ${slice.map(c => `<div style="background:#111;padding:4px">
      <div style="padding:2px 0;color:#ffd700">#${c.id} ${c.cls} ${c.page}</div>
      <img src="file://${process.cwd()}/${c.file}" style="max-width:100%;max-height:300px;display:block;background:#fff">
    </div>`).join('')}
    </div></body></html>`;
    fs.writeFileSync(`out/sheet${s}.html`, html);
    const ctx = await browser.newContext({ viewport: { width: 1100, height: 1000 }, deviceScaleFactor: 1 });
    const page = await ctx.newPage();
    await page.goto('file://' + process.cwd() + `/out/sheet${s}.html`, { waitUntil: 'load' });
    await page.waitForTimeout(600);
    await page.screenshot({ path: `out/sheet${s}.png`, fullPage: true });
    await ctx.close();
    console.log('sheet', s, slice.map(c => c.id).join(','));
  }
  await browser.close();
}
main();
