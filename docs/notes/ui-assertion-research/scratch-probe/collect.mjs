// THROWAWAY PROBE — Facts collector, shared by Phase 1 and Phase 2.
// Runs inside the page. Returns a serializable Facts snapshot that is
// intended to be a pure function of (archive, viewport, settle protocol).
export const COLLECTOR = `(() => {
  const MAX = 2500;
  const SP = ['display','position','fontSize','fontFamily','fontWeight','lineHeight','letterSpacing',
    'color','backgroundColor','backgroundImage','opacity','visibility','overflowX','overflowY',
    'marginTop','marginRight','marginBottom','marginLeft','paddingTop','paddingRight','paddingBottom','paddingLeft',
    'borderTopWidth','borderRadius','zIndex','flexDirection','gap','textAlign','whiteSpace','boxSizing','transform'];
  const r2 = n => Math.round(n * 100) / 100;
  const path = (el) => {
    const parts = [];
    let e = el;
    while (e && e.nodeType === 1 && e !== document.documentElement) {
      let i = 1, s = e;
      while ((s = s.previousElementSibling)) i++;
      parts.unshift(e.tagName.toLowerCase() + ':' + i);
      e = e.parentElement;
    }
    return 'html/' + parts.join('/');
  };
  const all = Array.from(document.querySelectorAll('*')).slice(0, MAX);
  const els = [];
  for (const el of all) {
    const tag = el.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'meta' || tag === 'link' || tag === 'head' || tag === 'title') continue;
    const b = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    const style = {};
    for (const k of SP) style[k] = cs[k];
    // direct text content only (not descendants)
    let ownText = '';
    for (const n of el.childNodes) if (n.nodeType === 3) ownText += n.nodeValue;
    ownText = ownText.replace(/\\s+/g, ' ').trim();
    let lineBoxes = null;
    if (ownText) {
      try { const rg = document.createRange(); rg.selectNodeContents(el); lineBoxes = rg.getClientRects().length; } catch (e) {}
    }
    els.push({
      p: path(el), tag,
      box: [r2(b.x), r2(b.y), r2(b.width), r2(b.height)],
      style,
      textLen: ownText.length,
      lineBoxes,
      vis: !!(el.offsetParent || cs.position === 'fixed') && cs.visibility !== 'hidden' && cs.display !== 'none' && parseFloat(cs.opacity) > 0,
      zi: cs.zIndex,
    });
  }
  return {
    meta: {
      vw: window.innerWidth, vh: window.innerHeight, dpr: window.devicePixelRatio,
      scrollW: document.documentElement.scrollWidth, scrollH: document.documentElement.scrollHeight,
      fonts: document.fonts ? document.fonts.status : 'n/a',
      total: document.querySelectorAll('*').length, captured: els.length,
    },
    els,
  };
})()`;

// Settle protocol P1. Applied before collection.
export async function settle(page, level) {
  if (level === 'naive') return;
  // freeze animation & transitions, disable smooth scrolling and caret
  await page.addStyleTag({
    content: `*,*::before,*::after{animation-play-state:paused!important;animation-delay:-0.0001s!important;
      animation-duration:0.0001s!important;transition-duration:0s!important;transition-delay:0s!important;
      caret-color:transparent!important;scroll-behavior:auto!important}`,
  }).catch(() => {});
  await page.evaluate(async () => {
    try { document.querySelectorAll('video').forEach(v => { v.pause?.(); v.currentTime = 0; }); } catch (e) {}
    try { document.getAnimations?.().forEach(a => { try { a.pause(); a.currentTime = 0; } catch (e) {} }); } catch (e) {}
    window.scrollTo(0, 0);
    if (document.fonts) { try { await document.fonts.ready; } catch (e) {} }
    try { await Promise.all([...document.images].filter(i => !i.complete).map(i => i.decode().catch(() => {}))); } catch (e) {}
    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
  await page.waitForTimeout(250);
}
