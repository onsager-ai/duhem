import { flowOrigin, iterationStepKey, type FlowOrigin, type VdLookup } from "./definition";
import type { StepNode, TimelineNode } from "./format";

export function stepNavigation(
  node: StepNode,
  vd?: VdLookup | null,
  criterionId?: string,
  checkId?: string,
) {
  const started = node.events[0];
  const cid = criterionId ?? (typeof started.criterion_id === "string" ? started.criterion_id : "");
  const chid = checkId ?? (typeof started.check_id === "string" ? started.check_id : "");
  const flow = flowOrigin(started.flow);
  const uses = typeof started.uses === "string" ? started.uses : "step";
  return {
    key: flow?.iteration !== undefined ? iterationStepKey(flow)
      : vd?.stepId(cid, chid, node.stepIndex, flow) ?? String(node.stepIndex),
    label: vd?.stepLabel(cid, chid, node.stepIndex, flow) ??
      (flow?.iteration === undefined ? `${uses} #${node.stepIndex}`
        : `${flow.invocation} › Iteration ${flow.iteration} › ${uses} #${node.stepIndex}`),
  };
}

export type LoopNode = {
  kind: "loop";
  key: string;
  origin: FlowOrigin;
  preamble?: StepNode;
  iterations: { ordinal: number; steps: StepNode[] }[];
};

/** Group a contiguous run of loop evidence by provenance, retaining first-seen
 * construct/iteration order. Point events and ordinary steps stay in place. */
export function groupLoops(nodes: TimelineNode[]): (TimelineNode | LoopNode)[] {
  const result: (TimelineNode | LoopNode)[] = [];
  let pending: StepNode[] = [];
  const flush = () => {
    const constructs = new Map<string, LoopNode>();
    for (const node of pending) {
      const origin = flowOrigin(node.events[0].flow)!;
      const key = JSON.stringify([origin.name, origin.invocation]);
      let construct = constructs.get(key);
      if (!construct) {
        construct = { kind: "loop", key: `loop-${node.key}`, origin, iterations: [] };
        constructs.set(key, construct);
      }
      let iteration = construct.iterations.find((item) => item.ordinal === origin.iteration);
      if (!iteration) {
        iteration = { ordinal: origin.iteration!, steps: [] };
        construct.iterations.push(iteration);
      }
      iteration.steps.push(node);
    }
    // No disclosure or reordering if this block contains no repeated construct.
    if ([...constructs.values()].every((group) => group.iterations.length === 1)) {
      result.push(...pending);
    } else {
      for (const group of constructs.values()) {
        if (group.iterations.length === 1) result.push(...group.iterations[0].steps);
        else {
          // Tier 1 records the loop's gate before its expanded body, with the
          // same authored index and lifecycle scope. Keep that evidence inside
          // the construct rather than presenting a second loop row beside it.
          const previous = result[result.length - 1];
          const started = group.iterations[0].steps[0].events[0];
          if (previous?.kind === "step" && previous.stepIndex === group.iterations[0].steps[0].stepIndex &&
              previous.events[0].kind === "setup_step_started" && !flowOrigin(previous.events[0].flow) &&
              ["phase", "criterion_id", "check_id", "fixture_name"].every((field) =>
                previous.events[0][field] === started[field])) {
            group.preamble = previous;
            result.pop();
          }
          result.push(group);
        }
      }
    }
    pending = [];
  };
  for (const node of nodes) {
    if (node.kind === "step" && flowOrigin(node.events[0].flow)?.iteration !== undefined) {
      pending.push(node);
    } else {
      flush();
      result.push(node);
    }
  }
  flush();
  return result;
}
