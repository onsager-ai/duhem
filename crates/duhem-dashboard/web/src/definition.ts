// Parse a recorded VD source snapshot (#302) into an id/index lookup, so
// the run views can overlay the *authored intent* — criterion/check
// descriptions and step ids — onto the execution trace by joining on
// `criterion_id` / `check_id` / `step_index`. Tolerant by construction:
// any shape that doesn't match yields an empty lookup (views fall back to
// ids), never a throw — a snapshot is evidence to read, not to trust.

import { parse } from "yaml";

export interface VdStep {
  id?: string;
  uses?: string;
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

export interface VdLookup {
  criterion(id: string): VdCriterion | undefined;
  check(criterionId: string, checkId: string): VdCheck | undefined;
  /** The author's `id:` for the Nth step of a check (0-based, matching
   *  the trace's `step_index`), or undefined if absent. */
  stepId(criterionId: string, checkId: string, index: number): string | undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function normStep(raw: unknown): VdStep {
  const r = (raw ?? {}) as Record<string, unknown>;
  return { id: str(r.id), uses: str(r.uses) };
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
  try {
    const doc = parse(yamlText) as { criteria?: unknown } | null;
    if (doc && Array.isArray(doc.criteria)) {
      criteria = doc.criteria.map(normCriterion).filter((c): c is VdCriterion => !!c);
    }
  } catch {
    criteria = [];
  }
  const byCrit = new Map(criteria.map((c) => [c.id, c]));
  const find = (cid: string, chid: string) =>
    byCrit.get(cid)?.checks.find((c) => c.id === chid);
  return {
    criterion: (id) => byCrit.get(id),
    check: (cid, chid) => find(cid, chid),
    stepId: (cid, chid, i) => find(cid, chid)?.steps[i]?.id,
  };
}
