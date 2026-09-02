// Parse a recorded VD source snapshot (#302) into an id/index lookup, so
// the run views can overlay the *authored intent* — criterion/check
// descriptions and step labels — onto the execution trace by joining on
// `criterion_id` / `check_id` / `step_index`. Tolerant by construction:
// any shape that doesn't match yields an empty lookup (views fall back to
// ids), never a throw — a snapshot is evidence to read, not to trust.

import { parse } from "yaml";

export interface VdStep {
  id?: string;
  description?: string;
  uses?: string;
  call?: string;
  with?: Record<string, unknown>;
}
export interface VdCheck {
  id: string;
  description?: string;
  steps: VdStep[];
}
export interface VdCriterion {
  id: string;
  description?: string;
  checks: VdCheck[];
}
interface VdFlow {
  description?: string;
  steps: VdStep[];
}

export interface VdLookup {
  criterion(id: string): VdCriterion | undefined;
  check(criterionId: string, checkId: string): VdCheck | undefined;
  /** The author's `id:` for the Nth step of a check (0-based, matching
   *  the trace's `step_index`), or undefined if absent. */
  stepId(
    criterionId: string,
    checkId: string,
    index: number,
    flow?: FlowOrigin,
  ): string | undefined;
  /** Human-facing label for the Nth step: description, then id, then
   *  `<uses> #<index>`. */
  stepLabel(
    criterionId: string,
    checkId: string,
    index: number,
    flow?: FlowOrigin,
  ): string | undefined;
  /** The author's `with:` map for the traced step, before runtime
   *  reference resolution. */
  stepWith(
    criterionId: string,
    checkId: string,
    index: number,
    flow?: FlowOrigin,
  ): Record<string, unknown> | undefined;
  /** Human-facing label for the authored inner step of a flow. */
  flowStepLabel(flow: FlowOrigin): string | undefined;
  /** Label for the authored invocation that owns an expanded step. */
  flowLabel(criterionId: string, checkId: string, flow: FlowOrigin): string | undefined;
}

export interface FlowOrigin {
  name: string;
  invocation: string;
  inner_index: number;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function record(v: unknown): Record<string, unknown> | undefined {
  return v !== null && typeof v === "object" && !Array.isArray(v)
    ? v as Record<string, unknown>
    : undefined;
}

function normStep(raw: unknown): VdStep {
  const r = (raw ?? {}) as Record<string, unknown>;
  return {
    id: str(r.id),
    description: str(r.description),
    uses: str(r.uses),
    call: str(r.call),
    with: record(r.with),
  };
}

function stepLabel(step: VdStep | undefined, index: number): string | undefined {
  return step?.description ??
    step?.id ??
    (step?.uses ? `${step.uses} #${index}` : step?.call);
}

export function flowOrigin(raw: unknown): FlowOrigin | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const value = raw as Record<string, unknown>;
  const name = str(value.name);
  const invocation = str(value.invocation);
  const inner = value.inner_index;
  return name && invocation && typeof inner === "number"
    ? { name, invocation, inner_index: inner }
    : undefined;
}

function normCheck(raw: unknown): VdCheck | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const r = raw as Record<string, unknown>;
  const id = str(r.id);
  if (!id) return undefined;
  return {
    id,
    description: str(r.description),
    steps: Array.isArray(r.steps) ? r.steps.map(normStep) : [],
  };
}

function normCriterion(raw: unknown): VdCriterion | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const r = raw as Record<string, unknown>;
  const id = str(r.id);
  if (!id) return undefined;
  return {
    id,
    description: str(r.description),
    checks: Array.isArray(r.checks)
      ? r.checks.map(normCheck).filter((c): c is VdCheck => !!c)
      : [],
  };
}

export function parseDefinition(yamlText: string): VdLookup {
  let criteria: VdCriterion[] = [];
  let flows = new Map<string, VdFlow>();
  try {
    const doc = parse(yamlText) as { criteria?: unknown; flows?: unknown } | null;
    if (doc && Array.isArray(doc.criteria)) {
      criteria = doc.criteria.map(normCriterion).filter((c): c is VdCriterion => !!c);
    }
    if (doc?.flows && typeof doc.flows === "object") {
      flows = new Map(
        Object.entries(doc.flows as Record<string, unknown>).map(([name, raw]) => {
          const body = (raw ?? {}) as Record<string, unknown>;
          return [name, {
            description: str(body.description),
            steps: Array.isArray(body.steps) ? body.steps.map(normStep) : [],
          }];
        }),
      );
    }
  } catch {
    criteria = [];
    flows = new Map();
  }
  const byCrit = new Map(criteria.map((c) => [c.id, c]));
  const find = (cid: string, chid: string) =>
    byCrit.get(cid)?.checks.find((c) => c.id === chid);
  const invocationStep = (cid: string, chid: string, origin: FlowOrigin) =>
    find(cid, chid)?.steps.find((step) => step.id === origin.invocation);
  const innerStep = (origin: FlowOrigin) =>
    flows.get(origin.name)?.steps[origin.inner_index];
  const invocationLabel = (cid: string, chid: string, origin: FlowOrigin) =>
    stepLabel(invocationStep(cid, chid, origin), origin.inner_index) ?? origin.invocation;
  const flowLabel = (cid: string, chid: string, origin: FlowOrigin) =>
    flows.get(origin.name)?.description ?? invocationLabel(cid, chid, origin);
  return {
    criterion: (id) => byCrit.get(id),
    check: (cid, chid) => find(cid, chid),
    stepId: (cid, chid, i, flow) => {
      if (flow) {
        const inner = innerStep(flow);
        return `${flow.invocation}__${inner?.id ?? flow.inner_index}`;
      }
      return find(cid, chid)?.steps[i]?.id;
    },
    stepLabel: (cid, chid, i, flow) => {
      if (flow) {
        const inner = stepLabel(innerStep(flow), flow.inner_index);
        const invocation = invocationLabel(cid, chid, flow);
        return inner ? `${invocation} › ${inner}` : invocation;
      }
      const step = find(cid, chid)?.steps[i];
      return stepLabel(step, i);
    },
    stepWith: (cid, chid, i, flow) =>
      (flow ? innerStep(flow) : find(cid, chid)?.steps[i])?.with,
    flowStepLabel: (flow) => stepLabel(innerStep(flow), flow.inner_index),
    flowLabel,
  };
}
