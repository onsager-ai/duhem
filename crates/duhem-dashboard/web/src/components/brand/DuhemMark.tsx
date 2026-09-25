import type { ReactNode, SVGProps } from "react";

// The Duhem mark, exactly as docs/duhem-brand.md §1 specifies it on the
// 32×32 grid: a continuous 4-unit frame (the auxiliary web) around a
// 10×10 center square (the hypothesis under test). Geometry is
// load-bearing (§8) — never scale the parts independently.
//
// Prefer 16 / 24 / 32 CSS px: 16 and 32 put every edge on a whole
// device pixel at 1×, and all three do at 2×.
//
// `FRAME` and `CENTER` are shared with `VerdictMark`, which keeps this
// geometry and only changes the center's fill (Appendix B display states).
export const FRAME = [
  { x: 2, y: 2, width: 28, height: 4 },
  { x: 2, y: 26, width: 28, height: 4 },
  { x: 2, y: 6, width: 4, height: 20 },
  { x: 26, y: 6, width: 4, height: 20 },
] as const;

export const CENTER = { x: 11, y: 11, width: 10, height: 10 } as const;

export function MarkSvg({
  center,
  ...props
}: SVGProps<SVGSVGElement> & { center?: ReactNode }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 32 32"
      {...props}
    >
      <g fill="currentColor">
        {FRAME.map((r) => (
          <rect key={`${r.x},${r.y}`} {...r} />
        ))}
      </g>
      {center ?? <rect fill="currentColor" {...CENTER} />}
    </svg>
  );
}

// The brand mark: monochrome, `currentColor`, no plate behind it
// (§8: the frame is already a container). Decorative wherever the
// wordmark sits next to it.
export function DuhemMark(props: SVGProps<SVGSVGElement>) {
  return <MarkSvg aria-hidden focusable="false" {...props} />;
}
