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

export interface CheckDetail {
  criterion_id: string;
  check_id: string;
  verdict: Verdict | null;
  timeline: TraceEvent[];
  artifacts: ArtifactRef[];
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

export function traceUrl(runId: string): string {
  return `api/runs/${encodeURIComponent(runId)}/trace.jsonl`;
}

export function liveUrl(runId: string): string {
  return `api/runs/${encodeURIComponent(runId)}/live`;
}
