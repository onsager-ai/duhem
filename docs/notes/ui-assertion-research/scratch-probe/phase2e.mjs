// THROWAWAY PROBE — Phase 2e: does the B-class false-positive rate survive
// a better geometric primitive?
//
// Adjudication of the border-box overlap check found 28/28 false positives.
// Every cause traced to the border box being the wrong primitive:
//   - a wrapped inline's rect is the union of its line boxes, so it covers
//     text belonging to other elements
//   - inline siblings on one line have touching/overlapping rects
//   - a container's rect covers its visually-nested (but not DOM-nested) peers
// Text does not collide with boxes; text collides with text. So compare the
// rendered *text run rectangles* (one per line box) instead.
import { chromium } from 'playwright';
import fs from 'node:fs';

const T = (p, ms, l) => Promise.race([Promise.resolve(p), new Promise((_, rj) => setTimeout(() => rj(new Error('TO ' + l)), ms))]);
const FREEZE = `(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;

// Collect one rect per rendered line of text, tagged with its owning element.
export const TEXTRUNS = `(() => {
  const runs = [];
  const path = (el) => { const p=[]; let e=el;
    while (e && e.nodeType===1 && e!==document.documentElement) { let i=1,s=e; while((s=s.previousElementSibling)) i++; p.unshift(e.tagName.toLowerCase()+':'+i); e=e.parentElement; }
    return 'html/'+p.join('/'); };
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  let n, id = 0;
  while ((n = walker.nextNode())) {
    const t = n.nodeValue;
    if (!t || !t.trim()) continue;
    const el = n.parentElement;
    if (!el) continue;
    const tag = el.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'noscript') continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || parseFloat(cs.opacity) === 0) continue;
    const r = document.createRange(); r.selectNodeContents(n);
    for (const b of r.getClientRects()) {
      if (b.width < 2 || b.height < 2) continue;
      runs.push({ id: id++, p: path(el), tag,
        box: [Math.round(b.x*100)/100, Math.round(b.y*100)/100, Math.round(b.width*100)/100, Math.round(b.height*100)/100],
        color: cs.color, bg: cs.backgroundColor, bgi: cs.backgroundImage !== 'none',
        pos: cs.position, zi: cs.zIndex, len: t.trim().length });
    }
  }
  // clip rects, so the observability gate can still run
  const clip = {};
  const seen = new Set(runs.map(r => r.p));
  for (const p of seen) {
    const parts = p.split('/').slice(1);
    let el = document.documentElement, ok = true;
    for (const part of parts) { const [tg, ix] = part.split(':'); el = el.children[Number(ix)-1]; if (!el || el.tagName.toLowerCase()!==tg) { ok=false; break; } }
    if (!ok || !el) continue;
    let x1=-Infinity,y1=-Infinity,x2=Infinity,y2=Infinity;
    for (let a = el.parentElement; a; a = a.parentElement) {
      const cs = getComputedStyle(a);
      if (cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
      const b = a.getBoundingClientRect();
      x1=Math.max(x1,b.x); y1=Math.max(y1,b.y); x2=Math.min(x2,b.right); y2=Math.min(y2,b.bottom);
    }
    clip[p] = [x1,y1,x2,y2];
  }
  return { runs, clip, vw: window.innerWidth };
})()`;

const isAnc = (a, b) => b.startsWith(a + '/');
const alphaOf = c => { const m=/rgba?\(([^)]+)\)/.exec(c||''); if(!m) return 1; const p=m[1].split(',').map(parseFloat); return p.length>3?p[3]:1; };

export function overlapCheck(data) {
  const { runs, clip } = data;
  const fail = [], incon = [], suppressed = [];
  const BAND = 100, bands = new Map();
  for (const r of runs) {
    const b0 = Math.floor(r.box[1]/BAND), b1 = Math.floor((r.box[1]+r.box[3])/BAND);
    for (let b=b0;b<=b1;b++){ if(!bands.has(b)) bands.set(b,[]); bands.get(b).push(r); }
  }
  const seen = new Set();
  for (const list of bands.values()) {
    for (let i=0;i<list.length;i++) for (let j=i+1;j<list.length;j++) {
      const a=list[i], b=list[j];
      if (a.p === b.p) continue;                       // same element, different line
      if (isAnc(a.p,b.p) || isAnc(b.p,a.p)) continue;  // text of an ancestor vs its child
      const k = a.id<b.id ? a.id+'|'+b.id : b.id+'|'+a.id;
      if (seen.has(k)) continue; seen.add(k);
      const ix = Math.min(a.box[0]+a.box[2], b.box[0]+b.box[2]) - Math.max(a.box[0], b.box[0]);
      const iy = Math.min(a.box[1]+a.box[3], b.box[1]+b.box[3]) - Math.max(a.box[1], b.box[1]);
      if (ix <= 2 || iy <= 2) continue;
      const ia = ix*iy, smaller = Math.min(a.box[2]*a.box[3], b.box[2]*b.box[3]);
      if (ia/smaller < 0.25) continue;                 // glyph rows must genuinely interpenetrate
      const rec = { a: a.p, b: b.p, ix:+ix.toFixed(1), iy:+iy.toFixed(1), cover:+(ia/smaller).toFixed(2),
                    box:[Math.max(a.box[0],b.box[0]), Math.max(a.box[1],b.box[1]), ix, iy] };
      // observability gate: is the collision region actually painted for both?
      const paints = (r) => { const c = clip[r.p]; if(!c) return true;
        const w=Math.min(rec.box[0]+ix,c[2])-Math.max(rec.box[0],c[0]);
        const h=Math.min(rec.box[1]+iy,c[3])-Math.max(rec.box[1],c[1]);
        return w>2 && h>2; };
      if (!paints(a) || !paints(b)) { suppressed.push({...rec, why:'collision-region-clipped'}); continue; }
      const top = (parseInt(a.zi)||0) !== (parseInt(b.zi)||0) ? ((parseInt(a.zi)||0)>(parseInt(b.zi)||0)?a:b) : (a.id>b.id?a:b);
      if (top.bgi) { incon.push({...rec, why:'occluder-bg-unresolvable'}); continue; }
      if (alphaOf(top.bg) === 1) { suppressed.push({...rec, why:'occluded-by-opaque-top'}); continue; }
      fail.push(rec);
    }
  }
  return { fail, incon, suppressed, runs: runs.length };
}

async function main(){
  const SHARD=Number(process.env.SHARD??0), NSHARD=Number(process.env.NSHARD??1);
  const prov=JSON.parse(fs.readFileSync('archives/provenance.json','utf8')).filter(r=>r.usable).filter((_,i)=>i%NSHARD===SHARD);
  const browser=await chromium.launch({headless:true});
  const all=[];
  for(const s of prov){
    const file=process.cwd()+'/archives/'+s.id+'.mhtml';
    if(!fs.existsSync(file)) continue;
    const ctx=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:1,locale:'en-US',timezoneId:'UTC',reducedMotion:'reduce'});
    await ctx.addInitScript(FREEZE);
    await ctx.route('**/*', r=>r.request().url().startsWith('file:')?r.continue():r.abort());
    const page=await ctx.newPage();
    try{
      await T(page.goto('file://'+file,{waitUntil:'load',timeout:40000}),45000,'goto');
      await page.evaluate(()=>{for(const s of document.querySelectorAll('svg')){try{s.setCurrentTime?.(0);s.pauseAnimations?.();}catch(e){}}
        try{for(const a of document.getAnimations?.()??[]){try{a.pause();a.currentTime=0;}catch(e){}}}catch(e){}});
      await page.waitForTimeout(400);
      const data=await T(page.evaluate(TEXTRUNS),40000,'runs');
      const r=overlapCheck(data);
      all.push({id:s.id,genre:s.genre,...r});
      console.log(s.id.padEnd(24),'runs',String(r.runs).padStart(5),'FAIL',String(r.fail.length).padStart(4),'incon',String(r.incon.length).padStart(3),'suppressed',String(r.suppressed.length).padStart(4));
    }catch(e){ console.log(s.id,'ERR',e.message.slice(0,50)); }
    await ctx.close().catch(()=>{});
    fs.writeFileSync(`out/phase2e.shard${SHARD}.json`,JSON.stringify(all,null,2));
  }
  await browser.close();
}
if (process.argv[1] && process.argv[1].endsWith('phase2e.mjs')) main();
