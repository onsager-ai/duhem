import { chromium } from 'playwright';
const b=await chromium.launch({headless:true});
const ctx=await b.newContext({viewport:{width:1280,height:800}});
await ctx.route('**/*', r=>r.request().url().startsWith('file:')?r.continue():r.abort());
const p=await ctx.newPage();
await p.goto('file://'+process.cwd()+'/archives/kubernetes.io.mhtml',{waitUntil:'load',timeout:40000});
const r=await p.evaluate(()=>{
  const svgs=[...document.querySelectorAll('svg')].filter(s=>s.getBoundingClientRect().width>2);
  const htmls=[...document.querySelectorAll('div')].filter(d=>d.getBoundingClientRect().width>2);
  return { svgCount:svgs.length,
    svgWithOffsetParent: svgs.filter(s=>s.offsetParent).length,
    svgOffsetParentType: typeof svgs[0]?.offsetParent,
    htmlWithOffsetParent: htmls.slice(0,50).filter(d=>d.offsetParent).length, htmlSampled:Math.min(50,htmls.length) };
});
console.log(JSON.stringify(r,null,1));
await b.close();
