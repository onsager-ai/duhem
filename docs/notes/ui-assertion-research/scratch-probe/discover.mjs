import { chromium } from 'playwright';
const T=(p,ms,l)=>Promise.race([Promise.resolve(p),new Promise((_,rj)=>setTimeout(()=>rj(new Error('TO '+l)),ms))]);
const SOURCES=['https://bolt.new/gallery/all','https://v0.app/community','https://v0.dev/community','https://lovable.dev/templates'];
const b=await chromium.launch({headless:true});
const found={};
for(const src of SOURCES){
  const ctx=await b.newContext({viewport:{width:1400,height:1000},locale:'en-US'});
  await ctx.route('**/*', async route=>{const rq=route.request();
    try{const r=await T(fetch(rq.url(),{method:rq.method(),headers:rq.headers(),redirect:'follow'}),20000,'f');
      const buf=Buffer.from(await r.arrayBuffer());const h={};
      for(const[k,v] of r.headers) if(!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k]=v;
      await route.fulfill({status:r.status,headers:h,body:buf});}catch{try{await route.abort();}catch{}}});
  const p=await ctx.newPage();
  try{
    const resp=await T(p.goto(src,{waitUntil:'load',timeout:45000}),50000,'goto');
    await p.waitForTimeout(3000);
    // scroll to trigger lazy grids
    await p.evaluate(async()=>{for(let y=0;y<4000;y+=800){window.scrollTo(0,y);await new Promise(r=>setTimeout(r,300));}});
    await p.waitForTimeout(1500);
    const links=await p.evaluate(()=>[...document.querySelectorAll('a[href]')].map(a=>a.href));
    const hits=[...new Set(links.filter(u=>/\.bolt\.host|\.lovable\.app|\.vercel\.app|v0\.app\/(chat|community)\/|v0\.dev\/(chat|community)\//.test(u)))];
    found[src]={status:resp?.status(), total:links.length, hits:hits.slice(0,60)};
    console.log(src,'status',resp?.status(),'links',links.length,'candidates',hits.length);
    for(const h of hits.slice(0,10)) console.log('   ',h);
  }catch(e){ console.log(src,'ERR',e.message.slice(0,60)); found[src]={err:e.message.slice(0,60)}; }
  await ctx.close().catch(()=>{});
}
(await import('node:fs')).writeFileSync('out/discover.json',JSON.stringify(found,null,2));
await b.close();
