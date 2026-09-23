import type { LifecycleBlock, LifecycleScopeSegment } from "./api";

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
