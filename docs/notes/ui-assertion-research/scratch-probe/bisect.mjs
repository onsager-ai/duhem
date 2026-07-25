import { chromium } from 'playwright';
const T=(p,ms,l)=>Promise.race([Promise.resolve(p),new Promise((_,rj)=>setTimeout(()=>rj(new Error('TO '+l)),ms))]);
const FREEZE=`(()=>{const inj=()=>{if(document.getElementById('__f'))return;const s=document.createElement('style');s.id='__f';s.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';(document.head||document.documentElement).appendChild(s);};if(document.documentElement)inj();document.addEventListener('DOMContentLoaded',inj,{once:true});})()`;
const b=await chromium.launch({headless:true});
async function scrollable(url,w){
  const ctx=await b.newContext({viewport:{width:w,height:900},deviceScaleFactor:1,locale:'en-US',timezoneId:'UTC',reducedMotion:'reduce'});
  await ctx.addInitScript(FREEZE);
  await ctx.route('**/*', async route=>{const rq=route.request();
    try{const r=await T(fetch(rq.url(),{method:rq.method(),headers:rq.headers(),redirect:'follow'}),20000,'f');
      const buf=Buffer.from(await r.arrayBuffer());const h={};
      for(const[k,v] of r.headers) if(!/^(content-encoding|content-length|transfer-encoding|content-security-policy.*)$/i.test(k)) h[k]=v;
      await route.fulfill({status:r.status,headers:h,body:buf});}catch{try{await route.abort();}catch{}}});
  const p=await ctx.newPage();
  try{ await T(p.goto(url,{waitUntil:'load',timeout:45000}),50000,'g'); await p.waitForTimeout(1300);
    const m=await p.evaluate(()=>{const b0=window.scrollX;window.scrollTo(9999,0);const a=window.scrollX;window.scrollTo(b0,0);return a;});
    return m>0; } finally { await ctx.close().catch(()=>{}); }
}
// find the boundary where the verdict flips, to +-2px
async function edge(url, lo, hi){ // lo=false side, hi=true side (or vice versa)
  let a=lo,c=hi;
  while (Math.abs(c-a)>2){ const mid=Math.round((a+c)/2);
    const v=await scrollable(url,mid);
    if (v===await Promise.resolve(true) && v) { c=mid; } else { a=mid; }
  }
  return { lower:a, upper:c };
}
const CASES=[
  {id:'news.ycombinator.com', url:'https://news.ycombinator.com/', lo:700, hi:768, hi2:840},
  {id:'allbirds.com', url:'https://www.allbirds.com/', lo:340, hi:320, hi2:null},
];
for(const c of CASES){
  // walk from the known-true width outward in fine steps to find both edges
  const marks={};
  const lowEdge=[], highEdge=[];
  for(let w=c.id.startsWith('news')?740:316; w<=(c.id.startsWith('news')?830:344); w+=4){
    marks[w]=await scrollable(c.url,w);
  }
  const trues=Object.entries(marks).filter(([,v])=>v).map(([w])=>+w);
  console.log(c.id,'observable window:', trues.length? `${Math.min(...trues)}–${Math.max(...trues)}px (width ${Math.max(...trues)-Math.min(...trues)+4}px)` : 'none');
  console.log('   fine grid:', Object.entries(marks).map(([w,v])=>w+(v?'✓':'·')).join(' '));
}
await b.close();
