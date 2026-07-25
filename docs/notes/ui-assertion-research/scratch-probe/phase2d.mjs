// THROWAWAY PROBE — Phase 2d: class-D cross-viewport sweep, LIVE.
// Archive replay is only faithful at its capture viewport (measured:
// stripe.com archive reports hScroll at 6 widths the live site does not),
// so the responsive sweep must hit the live page at each width.
import { chromium } from 'playwright';
import fs from 'node:fs';
const T=(p,ms,l)=>Promise.race([Promise.resolve(p),new Promise((_,rj)=>setTimeout(()=>rj(new Error('TO '+l)),ms))]);
const WIDTHS=[320,375,414,768,1024,1280,1440,1920];
const FREEZE=`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;
const MEASURE=`(()=>{
  const vw=window.innerWidth, out={vw, sw:document.documentElement.scrollWidth};
  out.hScroll = out.sw > vw+1;
  const offenders=[];
  for (const el of document.querySelectorAll('*')) {
    const b=el.getBoundingClientRect();
    if (b.width<=0||b.height<=0) continue;
    if (b.right > vw+1) {
      // is it actually clipped by an ancestor?
      let clipped=false;
      for (let a=el.parentElement; a; a=a.parentElement) {
        const cs=getComputedStyle(a);
        if (cs.overflowX!=='visible'||cs.overflowY!=='visible') {
          const ab=a.getBoundingClientRect();
          if (b.right > ab.right+1) { clipped=true; break; }
        }
      }
      if (!clipped) offenders.push({t:el.tagName.toLowerCase(),c:(typeof el.className==='string'?el.className.slice(0,30):''),r:Math.round(b.right)});
    }
  }
  out.unclippedOverflow = offenders.length;
  out.topOffenders = offenders.sort((a,b)=>b.r-a.r).slice(0,3);
  // declared breakpoints
  const bps=new Set();
  const walk=(rules)=>{for(const r of rules){ if(r.media||r.conditionText){
      for(const m of ((r.conditionText||r.media.mediaText||'')).matchAll(/(min|max)-width\\s*:\\s*([0-9.]+)(px|em|rem)/g)){
        let v=parseFloat(m[2]); if(m[3]!=='px') v*=16; bps.add(m[1]==='max'?Math.round(v)+1:Math.round(v)); } }
    if(r.cssRules) try{walk(r.cssRules);}catch(e){} }};
  for(const s of document.styleSheets){try{walk(s.cssRules);}catch(e){}}
  out.declared=[...bps].sort((a,b)=>a-b);
  return out;
})()`;

async function main(){
  const SHARD=Number(process.env.SHARD??0), NSHARD=Number(process.env.NSHARD??1);
  const REPEAT=process.env.REPEAT==='1';
  const prov=JSON.parse(fs.readFileSync('archives/provenance.json','utf8')).filter(r=>r.usable).filter((_,i)=>i%NSHARD===SHARD);
  const browser=await chromium.launch({headless:true});
  const out=[];
  for(const s of prov){
    const row={id:s.id,genre:s.genre,url:s.url,widths:{},declared:[]};
    for(const w of WIDTHS){
      const ctx=await browser.newContext({viewport:{width:w,height:900},deviceScaleFactor:1,locale:'en-US',timezoneId:'UTC',reducedMotion:'reduce'});
      await ctx.addInitScript(FREEZE);
      await ctx.route('**/*', async route=>{const rq=route.request();
        try{const r=await T(fetch(rq.url(),{method:rq.method(),headers:rq.headers(),redirect:'follow'}),20000,'f');
          const b=Buffer.from(await r.arrayBuffer()); const h={};
          for(const[k,v] of r.headers) if(!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k]=v;
          await route.fulfill({status:r.status,headers:h,body:b});}catch{try{await route.abort();}catch{}}});
      const page=await ctx.newPage();
      try{
        await T(page.goto(s.url,{waitUntil:'load',timeout:50000}),55000,'goto');
        await page.waitForTimeout(1500);
        const m=await T(page.evaluate(MEASURE),30000,'measure');
        if(!row.declared.length) row.declared=m.declared;
        delete m.declared;
        row.widths[w]=m;
      }catch(e){ row.widths[w]={err:e.message.slice(0,40)}; }
      await ctx.close().catch(()=>{});
    }
    const sig=w=>{const d=row.widths[w];return d&&!d.err?`${d.hScroll}|${d.unclippedOverflow>0}`:'err';};
    row.changePoints=[];
    for(let i=1;i<WIDTHS.length;i++){
      if(sig(WIDTHS[i])!==sig(WIDTHS[i-1])&&sig(WIDTHS[i])!=='err'&&sig(WIDTHS[i-1])!=='err')
        row.changePoints.push({from:WIDTHS[i-1],to:WIDTHS[i],a:sig(WIDTHS[i-1]),b:sig(WIDTHS[i])});
    }
    row.unexplained=row.changePoints.filter(cp=>!row.declared.some(d=>d>cp.from&&d<=cp.to));
    row.hScrollWidths=WIDTHS.filter(w=>row.widths[w]?.hScroll);
    row.overflowWidths=WIDTHS.filter(w=>row.widths[w]?.unclippedOverflow>0);
    out.push(row);
    console.log(s.id.padEnd(24),'hScroll@',(row.hScrollWidths.join(',')||'-').padEnd(22),'unclipOverflow@',(row.overflowWidths.join(',')||'-').padEnd(22),'chg',row.changePoints.length,'unexpl',row.unexplained.length);
    fs.writeFileSync(`out/phase2d${REPEAT?'r':''}.shard${SHARD}.json`,JSON.stringify(out,null,2));
  }
  await browser.close();
}
main();
