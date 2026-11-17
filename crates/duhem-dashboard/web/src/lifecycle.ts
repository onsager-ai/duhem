import type { LifecycleBlock, LifecycleScopeSegment, RunDetail } from "./api";

export function lifecycleScopePath(block: Pick<LifecycleBlock, "scope">): string {
  return block.scope.length === 0
    ? "leaf"
    : block.scope.map((segment) => `${segment.kind}:${segment.id}`).join(" / ");
}

export function lifecycleKey(block: LifecycleBlock): string {
  return String(block.timeline[0]?.seq ?? `${block.started_at}:${block.phase}:${lifecycleScopePath(block)}`);
}

export function lifecycleHref(runId: string, block: LifecycleBlock): string {
  return `/run/${encodeURIComponent(runId)}/lifecycle/${encodeURIComponent(lifecycleKey(block))}`;
}

export function lifecycleFailure(block: LifecycleBlock): string | undefined {
  return block.failing_step === undefined ? undefined : block.steps[block.failing_step]?.detail;
}

export function lifecycleEnclosesCheck(
  scope: LifecycleScopeSegment[], criterionId: string, checkId: string,
): boolean {
  const at = (index: number, kind: string, id?: string) =>
    scope[index]?.kind === kind && (id === undefined || scope[index]?.id === id);
  if (scope.length === 0) return true;
  if (scope.length === 1) return at(0, "criterion", criterionId) || at(0, "check", checkId);
  if (scope.length === 2) return (
    (at(0, "criterion", criterionId) && at(1, "check", checkId)) ||
    (at(0, "check", checkId) && at(1, "fixture")));
  if (scope.length === 3) return (
    at(0, "criterion", criterionId) && at(1, "check", checkId) && at(2, "fixture"));
  return false;
}

export function lifecycleAtCriterion(block: LifecycleBlock, criterionId: string): boolean {
  return block.scope.length === 1 &&
    block.scope[0].kind === "criterion" && block.scope[0].id === criterionId;
}

export function lifecycleAtCheck(block: LifecycleBlock, criterionId: string, checkId: string): boolean {
  return block.scope.length > 0 &&
    !lifecycleAtCriterion(block, criterionId) &&
    lifecycleEnclosesCheck(block.scope, criterionId, checkId);
}

/** The page that shows `block`: its own lifecycle route for leaf and
 *  criterion blocks, or the enclosing check with `?lifecycle=` for blocks
 *  scoped inside a check (the lifecycle route only serves the former). */
export function lifecycleTarget(run: Pick<RunDetail, "run_id" | "criteria">, block: LifecycleBlock): string {
  if (block.scope.length <= 1) return lifecycleHref(run.run_id, block);
  for (const criterion of run.criteria) {
    const check = criterion.checks.find((item) => lifecycleAtCheck(block, criterion.id, item.id));
    if (check) {
      const pair = encodeURIComponent(`${criterion.id}::${check.id}`);
      return `/run/${encodeURIComponent(run.run_id)}/check/${pair}?lifecycle=${encodeURIComponent(lifecycleKey(block))}`;
    }
  }
  return `/run/${encodeURIComponent(run.run_id)}/results`;
}

/** Recorded lifecycle blocks that did not pass, split by what they gate:
 *  a failed setup stops the work it encloses, and a failed `required`
 *  cleanup fails the run gate (spec §10.3.5). Diagnostic cleanup is
 *  evidence only and is left out. Read straight from the recorded block
 *  status and policy; nothing is re-judged. */
export function lifecycleFailures(run: Pick<RunDetail, "lifecycle">): {
  setup: LifecycleBlock[];
  requiredCleanup: LifecycleBlock[];
} {
  const failed = (run.lifecycle ?? []).filter((block) => block.status !== "passed");
  return {
    setup: failed.filter((block) => block.phase === "setup"),
    requiredCleanup: failed.filter(
      (block) => block.phase === "teardown" && block.cleanup_policy === "required",
    ),
  };
}
