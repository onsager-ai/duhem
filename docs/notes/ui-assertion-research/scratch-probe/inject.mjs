// THROWAWAY PROBE — fault injection primitives, shared by Phase 5a.
// Kept separate from the detectors so it is obvious nothing here touches them.

// Candidate enumeration + injection, evaluated in the page.
// `only` pins the injection to one element path so a severity ladder is
// measured on a constant element. Without it, the first candidate whose
// injection actually produces a non-zero geometric delta is chosen —
// necessary because e.g. min-width on a child of a shrink-to-fit parent
// widens the parent instead of protruding.
export const INJECT = `((cfg) => {
  const { fault, sev, only } = cfg;
  const path = (el) => { const p=[]; let e=el;
    while (e && e.nodeType===1 && e!==document.documentElement) { let i=1,s=e; while((s=s.previousElementSibling)) i++; p.unshift(e.tagName.toLowerCase()+':'+i); e=e.parentElement; }
    return 'html/'+p.join('/'); };
  const byPath = (p) => { const parts=p.split('/').slice(1); let el=document.documentElement;
    for (const part of parts) { const [tg,ix]=part.split(':'); el=el.children[Number(ix)-1];
      if (!el || el.tagName.toLowerCase()!==tg) return null; } return el; };
  const vis = (el) => { const cs=getComputedStyle(el); const b=el.getBoundingClientRect();
    return cs.visibility!=='hidden' && cs.display!=='none' && parseFloat(cs.opacity)>0 && b.width>2 && b.height>2; };
  const ownText = (el) => { let t=''; for (const n of el.childNodes) if (n.nodeType===3) t+=n.nodeValue; return t.trim(); };
  const BLOCKISH = /^(block|flex|grid|inline-block|list-item|table)$/;
  const unclipped = (el) => { for (let a=el.parentElement; a; a=a.parentElement) {
      const cs=getComputedStyle(a); if (cs.overflowX!=='visible'||cs.overflowY!=='visible') return false; } return true; };

  const all = Array.from(document.querySelectorAll('body *'));
  const candidates = [];
  if (fault === 'overlap') {
    for (const el of all) {
      if (!vis(el) || ownText(el).length < 8) continue;
      const sib = el.nextElementSibling;
      if (!sib || !vis(sib) || ownText(sib).length < 8) continue;
      const a = el.getBoundingClientRect(), b = sib.getBoundingClientRect();
      if (b.top < a.bottom - 1) continue;
      if (a.height > 200 || b.height > 200) continue;
      if (getComputedStyle(el).position !== 'static') continue;
      candidates.push(el);
    }
  } else if (fault === 'protrude') {
    for (const el of all) {
      if (!vis(el)) continue;
      const par = el.parentElement; if (!par || par === document.body) continue;
      if (getComputedStyle(par).overflowX !== 'visible') continue;
      if (!unclipped(el)) continue;   // else the protrusion is genuinely not painted
      const ecs = getComputedStyle(el);
      if (ecs.position !== 'static' || !BLOCKISH.test(ecs.display)) continue;
      const pb = par.getBoundingClientRect(), eb = el.getBoundingClientRect();
      if (pb.width < 200 || eb.width < 40 || eb.right > pb.right - 2) continue;
      candidates.push(el);
    }
  } else {
    for (const el of all) {
      if (!vis(el)) continue;
      const b = el.getBoundingClientRect();
      if (b.width < 120 || b.right > window.innerWidth - 2) continue;
      const ecs = getComputedStyle(el);
      if (ecs.position !== 'static' || !BLOCKISH.test(ecs.display)) continue;
      if (!unclipped(el)) continue;
      candidates.push(el);
    }
  }

  // Calibrated injection: aim for an ACHIEVED delta of exactly sev px, so the
  // severity ladder means the same thing on every page. An uncalibrated
  // shift-by-sev lands nowhere near sev once the element's natural gap or
  // box-sizing is involved (measured: 1px requested -> 632px achieved).
  const runRects = (n) => { const out=[];
    for (const c of n.childNodes) if (c.nodeType===3 && c.nodeValue.trim()) {
      const r=document.createRange(); r.selectNodeContents(c);
      for (const b of r.getClientRects()) if (b.width>2 && b.height>2) out.push(b); }
    return out; };
  const apply = (el) => {
    if (sev <= 0) return;
    if (fault === 'overlap') {
      // vertical gap between this element's last glyph row and the sibling's first
      const sib = el.nextElementSibling; if (!sib) return;
      const mine = runRects(el), theirs = runRects(sib);
      if (!mine.length || !theirs.length) return;
      const myBottom = Math.max(...mine.map(r => r.bottom));
      const theirTop = Math.min(...theirs.map(r => r.top));
      const gap = theirTop - myBottom;
      el.style.setProperty('position','relative','important');
      el.style.setProperty('top', (gap + sev)+'px','important');
    } else {
      el.style.setProperty('box-sizing','border-box','important');
      const b0 = el.getBoundingClientRect();
      const edge = fault === 'protrude'
        ? el.parentElement.getBoundingClientRect().right
        : window.innerWidth;
      el.style.setProperty('width', (b0.width + (edge - b0.right) + sev)+'px','important');
    }
  };
  const undo = (el) => { el.style.removeProperty('position'); el.style.removeProperty('top');
    el.style.removeProperty('min-width'); el.style.removeProperty('width'); el.style.removeProperty('box-sizing'); };
  const measure = (el) => {
    const tb = el.getBoundingClientRect();
    if (fault === 'overlap') {
      // Ground truth must be measured in the same currency the predicate uses:
      // glyph-row (text-run) intersection, not border-box intersection. A box
      // can overlap purely through padding with no text collision at all.
      const s = el.nextElementSibling; if (!s) return 0;
      const runsOf = (n) => { const out=[];
        for (const c of n.childNodes) if (c.nodeType===3 && c.nodeValue.trim()) {
          const r=document.createRange(); r.selectNodeContents(c);
          for (const b of r.getClientRects()) if (b.width>2 && b.height>2) out.push(b); }
        return out; };
      let best = 0;
      for (const a of runsOf(el)) for (const b of runsOf(s)) {
        const ix = Math.min(a.right,b.right) - Math.max(a.left,b.left);
        const iy = Math.min(a.bottom,b.bottom) - Math.max(a.top,b.top);
        if (ix > 2 && iy > 0) best = Math.max(best, iy);
      }
      return best;
    }
    if (fault === 'protrude') { const pb = el.parentElement.getBoundingClientRect();
      return Math.max(0, tb.right - pb.right); }
    return Math.max(0, tb.right - window.innerWidth);
  };

  if (only) {
    const el = byPath(only);
    if (!el) return { skipped: 'target-vanished' };
    apply(el);
    return { target: only, achieved: +measure(el).toFixed(2) };
  }
  // probe mode: choose the candidate that responds most strongly at this
  // severity, so the ladder below it has headroom. Picking merely the first
  // responder pins a marginal element and understates recall at low severity.
  let bestEl = null, bestA = 0;
  for (const el of candidates.slice(0, 150)) {
    if (measure(el) > 0.5) continue;   // pre-existing fault: would contaminate baseline
    apply(el);
    const a = measure(el);
    if (a > bestA) { bestA = a; bestEl = el; }
    undo(el);
  }
  if (!bestEl || bestA <= 0.5) return { skipped: candidates.length ? 'injection-had-no-effect' : 'no-eligible-target' };
  apply(bestEl);
  return { target: path(bestEl), achieved: +measure(bestEl).toFixed(2), probed: true, candidates: candidates.length };
})`;
