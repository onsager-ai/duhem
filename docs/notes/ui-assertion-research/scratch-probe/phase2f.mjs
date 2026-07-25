// THROWAWAY PROBE — adjudication evidence for the text-run overlap check.
import { chromium } from 'playwright';
import fs from 'node:fs';
const T=(p,ms,l)=>Promise.race([Promise.resolve(p),new Promise((_,rj)=>setTimeout(()=>rj(new Error('TO '+l)),ms))]);
const FREEZE=`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;
const shards=fs.readdirSync('out').filter(f=>/^phase2e\.shard/.test(f));
let all=[]; for(const f of shards) all=all.concat(JSON.parse(fs.readFileSync('out/'+f,'utf8')));
// stratified: up to 4 per page, pages with fails, deterministic
const cases=[];
for(const p of all.filter(p=>p.fail.length).sort((a,b)=>a.id.localeCompare(b.id))){
  const step=Math.max(1,Math.floor(p.fail.length/4));
  for(let i=0,k=0;i<p.fail.length&&k<4;i+=step,k++) cases.push({page:p.id,...p.fail[i]});
}
console.log('cases',cases.length);
const browser=await chromium.launch({headless:true});
fs.mkdirSync('out/crops2',{recursive:true});
const index=[]; let n=0;
const byPage=new Map(); for(const c of cases){ if(!byPage.has(c.page)) byPage.set(c.page,[]); byPage.get(c.page).push(c); }
for(const [pid,list] of byPage){
  const file=process.cwd()+'/archives/'+pid+'.mhtml';
  const ctx=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:2,locale:'en-US',timezoneId:'UTC',reducedMotion:'reduce'});
  await ctx.addInitScript(FREEZE);
  await ctx.route('**/*',r=>r.request().url().startsWith('file:')?r.continue():r.abort());
  const page=await ctx.newPage();
  try{
    await T(page.goto('file://'+file,{waitUntil:'load',timeout:40000}),45000,'goto');
    await page.evaluate(()=>{for(const s of document.querySelectorAll('svg')){try{s.setCurrentTime?.(0);s.pauseAnimations?.();}catch(e){}}
      try{for(const a of document.getAnimations?.()??[]){try{a.pause();a.currentTime=0;}catch(e){}}}catch(e){}});
    await page.waitForTimeout(400);
    for(const c of list){
      n++; const id=String(n).padStart(2,'0');
      const [x,y,w,h]=c.box;
      await page.evaluate(({x,y,w,h})=>{
        const d=document.createElement('div'); d.id='__mark';
        Object.assign(d.style,{position:'absolute',left:(x+scrollX)+'px',top:(y+scrollY)+'px',width:w+'px',height:h+'px',
          outline:'2px solid red',background:'rgba(255,0,0,0.18)',zIndex:2147483647,pointerEvents:'none'});
        document.body.appendChild(d);
      },{x,y,w,h});
      const clip={x:Math.max(0,x-70),y:Math.max(0,y-40),width:Math.min(700,w+140),height:Math.min(200,h+80)};
      try{ await page.screenshot({path:`out/crops2/${id}.png`,fullPage:true,clip});
           index.push({id,page:pid,...c}); }catch(e){ console.log('shotfail',id,e.message.slice(0,30)); }
      await page.evaluate(()=>document.getElementById('__mark')?.remove());
    }
  }catch(e){console.log('pagefail',pid,e.message.slice(0,40));}
  await ctx.close().catch(()=>{});
}
fs.writeFileSync('out/adj2-index.json',JSON.stringify(index,null,2));
const per=6;
for(let s=0;s*per<index.length;s++){
  const sl=index.slice(s*per,s*per+per);
  const html=`<html><body style="margin:0;background:#222;font:12px monospace;color:#fff"><div style="display:grid;grid-template-columns:1fr 1fr;gap:6px;padding:6px">
  ${sl.map(c=>`<div style="background:#111;padding:4px"><div style="color:#ffd700">#${c.id} ${c.page} cover=${c.cover}</div>
  <img src="file://${process.cwd()}/out/crops2/${c.id}.png" style="max-width:100%;max-height:230px;display:block;background:#fff"></div>`).join('')}
  </div></body></html>`;
  fs.writeFileSync(`out/s2_${s}.html`,html);
  const ctx=await browser.newContext({viewport:{width:1200,height:900},deviceScaleFactor:1});
  const pg=await ctx.newPage(); await pg.goto('file://'+process.cwd()+`/out/s2_${s}.html`,{waitUntil:'load'});
  await pg.waitForTimeout(500); await pg.screenshot({path:`out/s2_${s}.png`,fullPage:true}); await ctx.close();
}
console.log('sheets',Math.ceil(index.length/per),'crops',index.length);
await browser.close();
