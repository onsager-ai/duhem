import { useLayoutEffect, useRef, type ReactNode } from "react";
import { stepStatus, type StepNode } from "../format";
import type { LoopNode } from "../step-presentation";

/** Native disclosures keep every iteration directly reachable by mouse or
 * keyboard. Open only failing/selected iterations; labels use evidence ordinals. */
export function LoopGroup({ group, selectedKey, renderStep, rail = false }: {
  group: LoopNode;
  selectedKey?: string;
  renderStep: (step: StepNode) => ReactNode;
  rail?: boolean;
}) {
  const root = useRef<HTMLDetailsElement>(null);
  const selectedIteration = group.iterations.find((iteration) =>
    iteration.steps.some((step) => step.key === selectedKey))?.ordinal;
  const selected = selectedIteration !== undefined || (selectedKey !== undefined && group.preamble?.key === selectedKey);
  const failed = group.iterations.filter((iteration) =>
    iteration.steps.some((step) => stepStatus(step).failed));
  useLayoutEffect(() => {
    if (!selected || !root.current) return;
    root.current.open = true;
    const target = root.current.querySelector<HTMLDetailsElement>(`[data-iteration="${selectedIteration}"]`);
    if (target) target.open = true;
  }, [selected, selectedIteration]);
  return (
    <details ref={root} className={rail ? "ml-2 border-l pl-2 text-xs" : "px-2 pb-2"}
      data-testid={rail ? "rail-loop-group" : "loop-group"}
      open={failed.length > 0 || selected}>
      <summary className="cursor-pointer py-2 font-medium">
        {group.origin.invocation}
        {group.origin.name !== "for_each" && <code className="ml-2">{group.origin.name}</code>}
        {" · "}{group.iterations.length} iterations
        <span className="ml-1 text-muted-foreground">(0-based)</span>
        {failed.length > 0 && <span className="ml-2">{failed.length} with failures</span>}
      </summary>
      {group.preamble && (rail ? renderStep(group.preamble)
        : <ol className="flow-step-list">{renderStep(group.preamble)}</ol>)}
      {group.iterations.map(({ ordinal, steps }) => {
        const failures = steps.filter((step) => stepStatus(step).failed);
        const content = steps.map(renderStep);
        return (
          <details key={ordinal} data-testid={rail ? "rail-loop-iteration" : "loop-iteration"}
            data-iteration={ordinal} className="ml-3 border-l pl-2"
            open={failures.length > 0 || ordinal === selectedIteration}>
            <summary className="cursor-pointer py-1">
              Iteration {ordinal}
              {failures.map((step) => <span key={step.key} className={`ml-2 tone-${stepStatus(step).tone}`}>
                {stepStatus(step).label}
              </span>)}
            </summary>
            {rail ? content : <ol className="flow-step-list">{content}</ol>}
          </details>
        );
      })}
    </details>
  );
}
