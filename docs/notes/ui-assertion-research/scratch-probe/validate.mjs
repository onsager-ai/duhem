import { chromium } from 'playwright';
const T=(p,ms,l)=>Promise.race([Promise.resolve(p),new Promise((_,rj)=>setTimeout(()=>rj(new Error('TO '+l)),ms))]);
const W=[320,375,414,768,1024,1280,1440,1920];
const FREEZE=`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;
const browser = await chromium.launch({headless:true});
async function probe(target, live) {
  const out={};
  for (const w of W) {
    const ctx = await browser.newContext({viewport:{width:w,height:900},deviceScaleFactor:1,locale:'en-US',timezoneId:'UTC',reducedMotion:'reduce'});
    await ctx.addInitScript(FREEZE);
    if (live) {
      await ctx.route('**/*', async route => { const rq=route.request();
        try { const r=await T(fetch(rq.url(),{method:rq.method(),headers:rq.headers(),redirect:'follow'}),20000,'f');
          const b=Buffer.from(await r.arrayBuffer()); const h={};
          for(const[k,v] of r.headers) if(!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k]=v;
          await route.fulfill({status:r.status,headers:h,body:b}); } catch { try{await route.abort();}catch{} } });
    } else {
      await ctx.route('**/*', r => r.request().url().startsWith('file:') ? r.continue() : r.abort());
    }
    const page = await ctx.newPage();
    try {
      await T(page.goto(live?target:'file://'+target,{waitUntil:'load',timeout:50000}),55000,'goto');
      await page.waitForTimeout(1200);
      const m = await page.evaluate(()=>({sw:document.documentElement.scrollWidth, vw:window.innerWidth,
        widest:(()=>{let best=null,bw=0;for(const el of document.querySelectorAll('*')){const b=el.getBoundingClientRect();
          if(b.width>0&&b.right>window.innerWidth+1&&b.right>bw){bw=b.right;best=el.tagName+'.'+(el.className&&typeof el.className==='string'?el.className.slice(0,40):'')}}
          return best?best+' @'+Math.round(bw):null})()}));
      out[w]={hScroll:m.sw>m.vw+1, sw:m.sw, vw:m.vw, widest:m.widest};
    } catch(e){ out[w]={err:e.message.slice(0,40)}; }
    await ctx.close().catch(()=>{});
  }
  return out;
}
const arch = await probe(process.cwd()+'/archives/stripe.com.mhtml', false);
const live = await probe('https://stripe.com/', true);
console.log('width | archive sw/vw hScroll | live sw/vw hScroll | agree');
for (const w of W) {
  const a=arch[w],l=live[w];
  console.log(String(w).padStart(5),'|',String(a.err||`${a.sw}/${a.vw} ${a.hScroll}`).padEnd(22),'|',String(l.err||`${l.sw}/${l.vw} ${l.hScroll}`).padEnd(22),'|', (a.hScroll===l.hScroll)?'yes':'NO');
}
console.log('\narchive widest offenders:'); for(const w of W) if(arch[w].widest) console.log(' ',w,arch[w].widest);
console.log('live widest offenders:'); for(const w of W) if(live[w].widest) console.log(' ',w,live[w].widest);
await browser.close();
