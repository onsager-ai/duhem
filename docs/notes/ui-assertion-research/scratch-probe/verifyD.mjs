import { chromium } from 'playwright';
const T=(p,ms,l)=>Promise.race([Promise.resolve(p),new Promise((_,rj)=>setTimeout(()=>rj(new Error('TO '+l)),ms))]);
const FREEZE=`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;
const CASES=[['https://www.figma.com/',1280],['https://www.figma.com/',375],['https://www.gymshark.com/',320],
  ['https://news.ycombinator.com/',768],['https://www.allbirds.com/',320],['https://en.wikipedia.org/wiki/Typography',768],
  ['https://app.diagrams.net/',1280]];
const b=await chromium.launch({headless:true});
console.log('url                                  w    sw   clientW  canScroll  maxScrollX  bodyOverflowX  htmlOverflowX');
for(const [url,w] of CASES){
  const ctx=await b.newContext({viewport:{width:w,height:900},deviceScaleFactor:1,locale:'en-US',timezoneId:'UTC',reducedMotion:'reduce'});
  await ctx.addInitScript(FREEZE);
  await ctx.route('**/*', async route=>{const rq=route.request();
    try{const r=await T(fetch(rq.url(),{method:rq.method(),headers:rq.headers(),redirect:'follow'}),20000,'f');
      const bf=Buffer.from(await r.arrayBuffer());const h={};
      for(const[k,v] of r.headers) if(!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k]=v;
      await route.fulfill({status:r.status,headers:h,body:bf});}catch{try{await route.abort();}catch{}}});
  const p=await ctx.newPage();
  try{
    await T(p.goto(url,{waitUntil:'load',timeout:50000}),55000,'goto');
    await p.waitForTimeout(1500);
    const m=await p.evaluate(()=>{
      const de=document.documentElement;
      const before=window.scrollX;
      window.scrollTo(9999,0);
      const after=window.scrollX;
      window.scrollTo(before,0);
      return {sw:de.scrollWidth, cw:de.clientWidth, maxX:after,
        bo:getComputedStyle(document.body).overflowX, ho:getComputedStyle(de).overflowX};
    });
    console.log(url.slice(0,36).padEnd(36), String(w).padStart(4), String(m.sw).padStart(5), String(m.cw).padStart(7),
      String(m.maxX>0).padStart(10), String(m.maxX).padStart(11), m.bo.padStart(14), m.ho.padStart(14));
  }catch(e){console.log(url,w,'ERR',e.message.slice(0,40));}
  await ctx.close().catch(()=>{});
}
await b.close();
