// Pure derivations over the runs list (`api/runs.json`) — powering the
// Overview KPIs / trend and the Verifications index. No backend call;
// everything the landing needs is already in the runs tree.

import type { RunStatus, RunsListEntry } from "./api";
import { verdictFamily } from "./ui";

export type Family = "pass" | "fail" | "inconclusive" | null;
export type FamilyCounts = Record<Exclude<Family, null>, number>;

export function countVerdictFamilies(
  verdicts: (string | null)[],
): FamilyCounts {
  const counts: FamilyCounts = { pass: 0, fail: 0, inconclusive: 0 };
  for (const verdict of verdicts) {
    const family = verdictFamily(verdict);
    if (family !== null) counts[family] += 1;
  }
  return counts;
}

function startedMs(e: RunsListEntry): number {
  return e.started_at ? Date.parse(e.started_at) || 0 : 0;
}

// Flatten the recorded lineage tree to every real run. Run-set parents
// are no longer synthetic rollups: they are addressable recorded runs
// and therefore count alongside their descendants.
export function flatLeaves(entries: RunsListEntry[]): RunsListEntry[] {
  const out: RunsListEntry[] = [];
  for (const e of entries) {
    const { children, ...run } = e;
    out.push({ ...run, kind: "leaf" });
    out.push(...flatLeaves(children ?? []));
  }
  return out;
}

export interface OverviewStats {
  total: number;
  pass: number;
  fail: number;
  inconclusive: number;
  running: number;
  passRate: number | null; // over decided runs (pass + fail + inconclusive)
  verifications: number;
  failingVerifications: number; // verifications whose latest run failed
  recent: RunsListEntry[]; // newest-first leaves
  trend: Family[]; // oldest → newest families of the most recent runs
}

export function computeStats(entries: RunsListEntry[]): OverviewStats {
  const leaves = flatLeaves(entries);
  const newestFirst = [...leaves].sort((a, b) => startedMs(b) - startedMs(a));

  const { pass, fail, inconclusive } = countVerdictFamilies(
    leaves.map((entry) => entry.verdict),
  );
  let running = 0;
  for (const e of leaves) {
    if (e.status === "running") running++;
  }
  const decided = pass + fail + inconclusive;

  const latestByVerification = new Map<string, RunsListEntry>();
  for (const e of newestFirst) {
    if (!latestByVerification.has(e.verification)) {
      latestByVerification.set(e.verification, e);
    }
  }
  let failingVerifications = 0;
  for (const e of latestByVerification.values()) {
    if (verdictFamily(e.verdict) === "fail") failingVerifications++;
  }

  return {
    total: leaves.length,
    pass,
    fail,
    inconclusive,
    running,
    passRate: decided ? pass / decided : null,
    verifications: latestByVerification.size,
    failingVerifications,
    recent: newestFirst.slice(0, 8),
    trend: newestFirst
      .slice(0, 24)
      .reverse()
      .map((e) => verdictFamily(e.verdict)),
  };
}

export interface VerificationSummary {
  name: string;
  runs: number;
  latest: RunsListEntry | null;
  recent: Family[]; // oldest → newest, up to 12
  status: RunStatus;
}

export function verificationSummaries(
  entries: RunsListEntry[],
): VerificationSummary[] {
  const byVerification = new Map<string, RunsListEntry[]>();
  for (const e of flatLeaves(entries)) {
    const list = byVerification.get(e.verification) ?? [];
    list.push(e);
    byVerification.set(e.verification, list);
  }
  const out: VerificationSummary[] = [];
  for (const [name, runs] of byVerification) {
    const newestFirst = [...runs].sort((a, b) => startedMs(b) - startedMs(a));
    out.push({
      name,
      runs: runs.length,
      latest: newestFirst[0] ?? null,
      recent: newestFirst
        .slice(0, 12)
        .reverse()
        .map((e) => verdictFamily(e.verdict)),
      status: newestFirst[0]?.status ?? "finished",
    });
  }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}
