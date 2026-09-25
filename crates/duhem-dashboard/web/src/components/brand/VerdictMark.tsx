import type { RunStatus, Verdict } from "@/api";
import { cn } from "@/lib/utils";
import { verdictFamily } from "@/ui";
import { CENTER, MarkSvg } from "./DuhemMark";

// Appendix B display states of the mark (docs/duhem-brand.md): the frame
// stays foreground ink; only the center square — the hypothesis under
// test — carries run state. Running: the center pulses 0.8 → 1.0 opacity
// at 1.5s (`.verdict-mark-pulse`, off under prefers-reduced-motion).
// Finished: the center takes the verdict color. These are display states
// for verdict contexts only — never use this as the brand mark.
//
// Accessibility: when the verdict is already announced as text next to
// the mark (the usual case — a VerdictBadge sits beside it), leave
// `label` unset and the mark is aria-hidden. Pass `label` only where the
// mark stands alone, and it becomes `role="img"` with that name.
const CENTER_FILL = {
  pass: "fill-pass",
  fail: "fill-fail",
  inconclusive: "fill-inconclusive",
} as const;

export function VerdictMark({
  verdict,
  status,
  label,
  className,
}: {
  verdict: Verdict | null;
  status?: RunStatus;
  label?: string;
  className?: string;
}) {
  const running = status === "running";
  const family = verdictFamily(verdict);
  const centerClass = running
    ? "fill-current verdict-mark-pulse"
    : family
      ? CENTER_FILL[family]
      : "fill-muted-foreground";
  const a11y = label
    ? ({ role: "img", "aria-label": label } as const)
    : ({ "aria-hidden": true } as const);
  return (
    <MarkSvg
      {...a11y}
      focusable="false"
      data-verdict={running ? "running" : (family ?? "none")}
      className={cn("shrink-0 text-foreground", className)}
      center={<rect className={centerClass} {...CENTER} />}
    />
  );
}
