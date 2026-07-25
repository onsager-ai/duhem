// THROWAWAY PROBE — not production code. Phase 0 corpus capture.
// Freezes real in-the-wild pages as self-contained MHTML archives that
// render offline, so Phase 1 reproducibility is measured against a fixed
// artifact rather than a mutating live site.
import { chromium } from 'playwright';
import fs from 'node:fs';

const VIEWPORT = { width: 1280, height: 800 };
const DPR = 1, LOCALE = 'en-US', TZ = 'UTC';

const TARGETS = [
  ['marketing', 'https://stripe.com/'],
  ['marketing', 'https://linear.app/'],
  ['marketing', 'https://vercel.com/'],
  ['marketing', 'https://www.figma.com/'],
  ['marketing', 'https://www.netlify.com/'],
  ['marketing', 'https://slack.com/'],
  ['app', 'https://excalidraw.com/'],
  ['app', 'https://codepen.io/'],
  ['app', 'https://play.grafana.org/'],
  ['app', 'https://squoosh.app/'],
  ['app', 'https://app.diagrams.net/'],
  ['docs', 'https://developer.mozilla.org/en-US/docs/Web/CSS/display'],
  ['docs', 'https://docs.python.org/3/library/json.html'],
  ['docs', 'https://react.dev/learn'],
  ['docs', 'https://tailwindcss.com/docs/flex'],
  ['docs', 'https://kubernetes.io/docs/concepts/overview/'],
  ['ecommerce', 'https://books.toscrape.com/'],
  ['ecommerce', 'https://www.allbirds.com/'],
  ['ecommerce', 'https://www.gymshark.com/'],
  ['ecommerce', 'https://www.rei.com/'],
  ['ecommerce', 'https://www.etsy.com/'],
  ['editorial', 'https://en.wikipedia.org/wiki/Typography'],
  ['editorial', 'https://news.ycombinator.com/'],
  ['editorial', 'https://www.bbc.com/news'],
  ['editorial', 'https://www.theguardian.com/international'],
  ['editorial', 'https://arstechnica.com/'],
  ['editorial', 'https://www.smashingmagazine.com/'],
  ['editorial', 'https://css-tricks.com/'],
];

// img.decode() never settles for images whose fetch failed -> every await needs a cap.
const T = (p, ms, label) => Promise.race([
  Promise.resolve(p),
  new Promise((_, rj) => setTimeout(() => rj(new Error('TIMEOUT ' + label)), ms)),
]);

const robotsCache = new Map();
async function robotsAllows(url) {
  const u = new URL(url);
  const origin = u.origin;
  if (!robotsCache.has(origin)) {
    let rules = [];
    try {
      const r = await fetch(origin + '/robots.txt', { redirect: 'follow' });
      if (r.ok) {
        const txt = await r.text();
        let inStar = false;
        for (const raw of txt.split('\n')) {
          const line = raw.split('#')[0].trim();
          if (!line) continue;
          const m = line.match(/^([A-Za-z-]+)\s*:\s*(.*)$/);
          if (!m) continue;
          const [, k, v] = [m[0], m[1].toLowerCase(), m[2].trim()];
          if (k === 'user-agent') inStar = v === '*';
          else if (inStar && k === 'disallow' && v) rules.push(v);
        }
      }
    } catch { /* no robots reachable => treat as allowed */ }
    robotsCache.set(origin, rules);
  }
  const path = u.pathname + u.search;
  return !robotsCache.get(origin).some(p => path.startsWith(p));
}

async function main() {
  const browser = await chromium.launch({ headless: true });
  const results = [];
  for (const [genre, url] of TARGETS) {
    const id = new URL(url).hostname.replace(/^www\./, '').replace(/[^a-z0-9.]/gi, '_');
    const rec = { id, genre, url, ok: false };
    try {
      if (!(await robotsAllows(url))) { rec.error = 'robots-disallow'; results.push(rec); console.log('SKIP(robots)', url); continue; }
      const ctx = await browser.newContext({
        viewport: VIEWPORT, deviceScaleFactor: DPR, locale: LOCALE, timezoneId: TZ,
      });
      let bytes = 0, reqs = 0;
      await ctx.route('**/*', async (route) => {
        const req = route.request();
        try {
          const r = await fetch(req.url(), { method: req.method(), headers: req.headers(), redirect: 'follow' });
          const buf = Buffer.from(await r.arrayBuffer());
          bytes += buf.length; reqs++;
          const h = {};
          for (const [k, v] of r.headers) if (!/^(content-encoding|content-length|transfer-encoding|content-security-policy|content-security-policy-report-only)$/i.test(k)) h[k] = v;
          await route.fulfill({ status: r.status, headers: h, body: buf });
        } catch { try { await route.abort(); } catch {} }
      });
      const page = await ctx.newPage();
      const resp = await T(page.goto(url, { waitUntil: 'load', timeout: 60000 }), 65000, 'goto');
      rec.status = resp?.status();
      rec.ua = await page.evaluate(() => navigator.userAgent);
      // settle for capture: fonts, lazy-load via full scroll, then home
      try { await T(page.evaluate(() => document.fonts?.ready), 10000, 'fonts'); } catch {}
      try {
        await T(page.evaluate(async () => {
          const step = window.innerHeight;
          const cap = Math.min(document.body.scrollHeight, 40000);
          for (let y = 0; y < cap; y += step) {
            window.scrollTo(0, y); await new Promise(r => setTimeout(r, 120));
          }
          window.scrollTo(0, 0);
        }), 40000, 'scroll');
      } catch {}
      await page.waitForTimeout(1200);
      try { await T(page.evaluate(() => Promise.all([...document.images].filter(i => !i.complete).map(i => i.decode().catch(() => {})))), 8000, 'decode'); } catch {}

      const rendered = await page.evaluate(() => document.querySelectorAll('*').length);
      let rawTags = 0;
      try { rawTags = ((await (await T(fetch(url), 20000, 'raw')).text()).match(/<[a-zA-Z][^>]*>/g) || []).length; } catch {}
      rec.renderedEls = rendered;
      rec.rawTags = rawTags;
      rec.jsRatio = rawTags ? +(rendered / rawTags).toFixed(2) : null;
      rec.spa = rawTags < 150 || rendered / Math.max(rawTags, 1) > 3;

      const cdp = await ctx.newCDPSession(page);
      const snap = await T(cdp.send('Page.captureSnapshot', { format: 'mhtml' }), 90000, 'cdp');
      fs.writeFileSync(`archives/${id}.mhtml`, snap.data);
      rec.archiveKB = Math.round(snap.data.length / 1024);
      rec.netKB = Math.round(bytes / 1024); rec.reqs = reqs;
      rec.capturedAt = new Date().toISOString();
      rec.ok = true;
      await ctx.close();
      console.log('OK', genre, id, `${rec.archiveKB}KB els=${rendered} spa=${rec.spa}`);
    } catch (e) {
      rec.error = e.message.split('\n')[0].slice(0, 120);
      console.log('FAIL', url, rec.error);
    }
    results.push(rec);
    fs.writeFileSync('archives/provenance.json', JSON.stringify(results, null, 2));
    await new Promise(r => setTimeout(r, 1500)); // rate limit
  }
  await browser.close();
  const ok = results.filter(r => r.ok);
  console.log(`\nCAPTURED ${ok.length}/${TARGETS.length}`);
  for (const g of [...new Set(TARGETS.map(t => t[0]))]) console.log(' ', g, ok.filter(r => r.genre === g).length);
}
main();
