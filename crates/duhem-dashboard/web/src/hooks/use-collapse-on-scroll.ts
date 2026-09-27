import { useEffect, useState, type RefObject } from "react";

// The run header's verdict band collapses to a single line once the
// reader scrolls (#563): the document on Summary and on mobile, or either
// Results pane (rail or detail) inside `root` on desktop. Scroll events
// do not bubble, so one capture-phase listener on the document sees them
// all.
//
// Hysteresis keeps the band from flickering: it collapses past
// `collapseAt`, and expands again only back at the very top. It does not
// collapse at all when the scroller has less than `minRunway` of travel:
// collapsing grows the pane, and a pane that then fits its content would
// clamp back to the top and bounce the band open again.
export function useCollapseOnScroll(
  root: RefObject<HTMLElement | null>,
  { collapseAt = 24, minRunway = 160 }: { collapseAt?: number; minRunway?: number } = {},
): boolean {
  const [collapsed, setCollapsed] = useState(false);
  useEffect(() => {
    const onScroll = (event: Event) => {
      const target = event.target;
      let scroller: Element;
      if (target === document || target === document.documentElement || target === document.body) {
        scroller = document.scrollingElement ?? document.documentElement;
      } else if (target instanceof Element && root.current?.contains(target)) {
        scroller = target;
      } else {
        return;
      }
      const top = scroller.scrollTop;
      const runway = scroller.scrollHeight - scroller.clientHeight;
      setCollapsed((was) => (was ? top > 0 : top > collapseAt && runway > minRunway));
    };
    document.addEventListener("scroll", onScroll, { capture: true, passive: true });
    return () => document.removeEventListener("scroll", onScroll, { capture: true });
  }, [root, collapseAt, minRunway]);
  return collapsed;
}
