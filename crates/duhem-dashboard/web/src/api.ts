// Fetch layer over the duhem-dashboard JSON API (#53 contract).
//
// Every path here is *relative* and carries a `.json` suffix so the
// same code works against the live server (which aliases the
// suffixed paths) and a static export (where the snapshots are real
// `.json` files). With hash routing the document URL never leaves
// the app root, so relative fetches resolve against the right base
// in both modes.

export type Verdict = string; // "pass" | "fail" | "inconclusive:<cause>"

export interface RunsListEntry {
  run_id: string;
  verification: string;
  started_at: string | null;
  duration_ms: number | null;
  verdict: Verdict | null;
  kind: "leaf" | "run-set";
  live: boolean;
  children?: RunsListEntry[];
}

export interface CheckRef {
  id: string;
  verdict: Verdict | null;
}

export interface CriterionDetail {
  id: string;
  verdict: Verdict | null;
  checks: CheckRef[];
}

export interface RunDetail {
  run_id: string;
  verification: string;
  started_at: string | null;
  inputs: Record<string, unknown>;
  verdict: Verdict | null;
  live: boolean;
  setup_aborted: boolean;
  /** `true` when the run recorded its VD source snapshot (#302); the
   *  client then fetches it from `definitionUrl`. */
  has_definition: boolean;
  criteria: CriterionDetail[];
}

export interface TraceEvent {
  seq: number;
  ts: string;
  kind: string;
  [key: string]: unknown;
}

export interface ArtifactRef {
  id: string;
  kind: string;
  url: string;
}

export interface SpanModel {
  seq: number;
  layer: string;
  ok: boolean;
  detail?: string;
}

export interface CheckDetail {
  criterion_id: string;
  check_id: string;
  verdict: Verdict | null;
  spans: SpanModel[];
  timeline: TraceEvent[];
  artifacts: ArtifactRef[];
}

export interface HistoryRun {
  run_id: string;
  started_at: string | null;
  verdict: Verdict | null;
  duration_ms: number | null;
}

export interface CriterionHistory {
  criterion_id: string;
  verdicts: (Verdict | null)[];
}

export interface VerificationHistory {
  name: string;
  runs: HistoryRun[];
  criteria: CriterionHistory[];
}

// #211 run-to-run diff.
export interface RunSide {
  run_id: string;
  started_at: string | null;
  verdict: Verdict | null;
}

export interface AssertionDiff {
  assertion_index: number;
  baseline_state: Verdict | null;
  current_state: Verdict | null;
  baseline_detail?: string;
  current_detail?: string;
  changed: boolean;
}

export interface CheckDiff {
  id: string;
  baseline_verdict: Verdict | null;
  current_verdict: Verdict | null;
  changed: boolean;
  assertions: AssertionDiff[];
  baseline_artifacts: ArtifactRef[];
  current_artifacts: ArtifactRef[];
}

export interface CriterionDiff {
  id: string;
  baseline_verdict: Verdict | null;
  current_verdict: Verdict | null;
  changed: boolean;
  checks: CheckDiff[];
}

export interface RunDiff {
  current: RunSide;
  baseline: RunSide | null;
  criteria: CriterionDiff[];
}

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path);
  if (!res.ok) {
    throw new Error(`GET ${path}: ${res.status}`);
  }
  return (await res.json()) as T;
}

export function fetchRuns(): Promise<RunsListEntry[]> {
  return getJson("api/runs.json");
}

export function fetchRun(runId: string): Promise<RunDetail> {
  return getJson(`api/runs/${encodeURIComponent(runId)}.json`);
}

export function fetchCheck(
  runId: string,
  criterionId: string,
  checkId: string,
): Promise<CheckDetail> {
  const pair = encodeURIComponent(`${criterionId}::${checkId}`);
  return getJson(`api/runs/${encodeURIComponent(runId)}/checks/${pair}.json`);
}

export function fetchVerificationHistory(
  name: string,
): Promise<VerificationHistory> {
  return getJson(`api/verifications/${encodeURIComponent(name)}/history.json`);
}

export function fetchDiff(runId: string, baseline?: string): Promise<RunDiff> {
  const q = baseline ? `?baseline=${encodeURIComponent(baseline)}` : "";
  return getJson(`api/runs/${encodeURIComponent(runId)}/diff.json${q}`);
}

export function traceUrl(runId: string): string {
  return `api/runs/${encodeURIComponent(runId)}/trace.jsonl`;
}

export function definitionUrl(runId: string): string {
  return `api/runs/${encodeURIComponent(runId)}/definition`;
}

/** The recorded VD source snapshot (raw YAML, #302). Throws on a run
 *  that has no snapshot (older runs) — callers gate on `has_definition`. */
export async function fetchDefinition(runId: string): Promise<string> {
  const res = await fetch(definitionUrl(runId));
  if (!res.ok) throw new Error(`GET definition: ${res.status}`);
  return res.text();
}

export function liveUrl(runId: string): string {
  return `api/runs/${encodeURIComponent(runId)}/live`;
}
