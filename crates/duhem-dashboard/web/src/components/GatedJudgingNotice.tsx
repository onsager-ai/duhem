/** Conditional absence is evidence, never an additional verdict state. */
export function GatedJudgingNotice({ count }: { count?: number }) {
  if (!count) return null;
  return (
    <span className="text-sm text-amber-700 dark:text-amber-400" data-testid="gated-judging-notice">
      {count} judging step{count === 1 ? "" : "s"} gated
    </span>
  );
}
