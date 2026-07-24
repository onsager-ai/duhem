// Run report (#86): the run summary panel — a check-verdict roll-up,
// run metadata, and inputs — rendered inside the shared RunScaffold
// tree (criteria → checks in the rail, this summary in the panel).
// Live runs (#84) fold their SSE stream in RunScaffold's `useRun`.

import { Link, useParams } from "react-router-dom";
import { traceUrl, type RunDetail } from "../api";
import { formatStartedAt } from "../ui";
import { RunScaffold } from "./RunScaffold";

// #280 Phase 2/3: an Allure-style status roll-up for a run — a donut of
// the check verdicts plus a count legend. Mechanically derived from the
// recorded criterion → check verdicts, never re-judged.
export interface StatusTally {
  pass: number;
  fail: number;
  inconclusive: number;
  pending: number;
  total: number;
}

function tallyVerdicts(verdicts: (string | null)[]): StatusTally {
  const t: StatusTally = { pass: 0, fail: 0, inconclusive: 0, pending: 0, total: 0 };
  for (const v of verdicts) {
    t.total++;
    if (v === "pass") t.pass += 1;
    else if (v === "fail") t.fail += 1;
    else if (v && v.startsWith("inconclusive")) t.inconclusive += 1;
    else t.pending += 1;
  }
  return t;
}

export function tallyChecks(criteria: RunDetail["criteria"]): StatusTally {
  return tallyVerdicts(
    criteria.flatMap((criterion) =>
      criterion.checks.map((check) => check.verdict),
    ),
  );
}

export function tallyCriteria(criteria: RunDetail["criteria"]): StatusTally {
  return tallyVerdicts(criteria.map((criterion) => criterion.verdict));
}

const DONUT_SEGMENTS: { key: keyof Omit<StatusTally, "total">; cls: string; label: string }[] = [
  { key: "pass", cls: "seg-pass", label: "passed" },
  { key: "fail", cls: "seg-fail", label: "failed" },
  { key: "inconclusive", cls: "seg-inconclusive", label: "inconclusive" },
  { key: "pending", cls: "seg-pending", label: "pending" },
];

// A pure-SVG donut: each status is an arc whose length is its share of
// the total, stacked by advancing the dash offset. Total in the middle.
export function StatusDonut({ tally }: { tally: StatusTally }) {
  const total = tally.total || 1;
  const r = 42;
  const circ = 2 * Math.PI * r;
  let offset = 0;
  const arcs = DONUT_SEGMENTS.map((s) => {
    const n = tally[s.key];
    if (n === 0) return null;
    const len = (n / total) * circ;
    const arc = (
      <circle
        key={s.key}
        className={`donut-seg ${s.cls}`}
        cx="50"
        cy="50"
        r={r}
        strokeDasharray={`${len} ${circ - len}`}
        strokeDashoffset={-offset}
      />
    );
    offset += len;
    return arc;
  });
  return (
    <div className="status-summary" data-testid="status-summary">
      <svg
        className="donut"
        viewBox="0 0 100 100"
        width="88"
        height="88"
        role="img"
        aria-label={`${tally.pass} passed, ${tally.fail} failed, ${tally.inconclusive} inconclusive of ${tally.total} checks`}
      >
        <circle className="donut-track" cx="50" cy="50" r={r} />
        {arcs}
        <text className="donut-center" x="50" y="52" textAnchor="middle">
          {tally.total}
        </text>
      </svg>
      <ul className="status-counts">
        {DONUT_SEGMENTS.map((s) => (
          <li key={s.key} className={`count ${s.cls}`} data-testid={`count-${s.key}`}>
            <span className="count-n">{tally[s.key]}</span> {s.label}
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function RunPage() {
  const { runId = "" } = useParams();
  return <RunScaffold runId={runId}>{(run) => <RunSummary run={run} />}</RunScaffold>;
}

// The Summary panel: a check-verdict roll-up, where the run came from,
// and the inputs it ran against. The per-check evidence lives one click
// away in the rail — this is the "what happened overall" view. Every
// number is derived mechanically from the recorded criterion → check
// verdicts (`tallyChecks`), never re-judged.
const SUMMARY_TILES: {
  key: keyof Omit<StatusTally, "total">;
  label: string;
  tone: string;
}[] = [
  { key: "pass", label: "Passed", tone: "text-pass" },
  { key: "fail", label: "Failed", tone: "text-fail" },
  { key: "inconclusive", label: "Inconclusive", tone: "text-inconclusive" },
  { key: "pending", label: "Pending", tone: "text-muted-foreground" },
];

const BAR_SEGMENTS: { key: keyof Omit<StatusTally, "total">; cls: string }[] = [
  { key: "pass", cls: "bg-pass" },
  { key: "fail", cls: "bg-fail" },
  { key: "inconclusive", cls: "bg-inconclusive" },
  { key: "pending", cls: "bg-foreground/25" },
];

function StatusBreakdown({
  label,
  tally,
}: {
  label: "Checks" | "Criteria";
  tally: StatusTally;
}) {
  const total = tally.total || 1;
  const id = `status-${label.toLowerCase()}`;
  return (
    <section aria-labelledby={id}>
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h3 id={id} className="text-sm font-semibold">
          {label}
        </h3>
        <span className="text-xs text-muted-foreground">
          {tally.total} total
        </span>
      </div>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {SUMMARY_TILES.map((tile) => (
          <div key={tile.key} data-testid={`${label.toLowerCase()}-${tile.key}`}>
            <div className={`text-2xl font-semibold tabular-nums ${tile.tone}`}>
              {tally[tile.key]}
            </div>
            <div className="text-xs text-muted-foreground">{tile.label}</div>
          </div>
        ))}
      </div>
      <div
        className="mt-3 flex h-2 overflow-hidden rounded-full bg-muted"
        role="img"
        aria-label={`${label}: ${tally.pass} passed, ${tally.fail} failed, ${tally.inconclusive} inconclusive, ${tally.pending} pending`}
      >
        {BAR_SEGMENTS.map((seg) =>
          tally[seg.key] > 0 ? (
            <div
              key={seg.key}
              className={seg.cls}
              style={{ width: `${(tally[seg.key] / total) * 100}%` }}
            />
          ) : null,
        )}
      </div>
    </section>
  );
}

function RunSummary({ run }: { run: RunDetail }) {
  const checks = tallyChecks(run.criteria);
  const criteria = tallyCriteria(run.criteria);
  const inputs = Object.entries(run.inputs);
  return (
    <div className="min-w-0 max-w-full space-y-5" data-testid="run-summary">
      {run.setup_aborted && (
        <div className="border-l-2 border-fail bg-fail/5 px-4 py-3 text-sm text-fail">
          Setup aborted — no checks ran. The verdict reflects the abort, not the
          artifact.
        </div>
      )}

      {/* Checks and criteria use different denominators. Keep both explicit:
          criteria without checks must remain visible instead of disappearing
          into the executable-check roll-up. */}
      <div className="space-y-4 border-y py-4">
        <StatusBreakdown label="Checks" tally={checks} />
        <div className="border-t pt-4">
          <StatusBreakdown label="Criteria" tally={criteria} />
        </div>
      </div>

      <p className="border-l-2 border-muted-foreground/30 px-3 py-1 text-sm text-muted-foreground">
        Select a check in the criteria navigator to inspect its failed steps,
        assertions, and evidence.
      </p>

      {/* Provenance + evidence links. */}
      <dl className="grid gap-x-6 gap-y-4 text-sm sm:grid-cols-2">
        <div>
          <dt className="mb-0.5 text-xs text-muted-foreground">Started</dt>
          <dd className="font-medium">{formatStartedAt(run.started_at)}</dd>
        </div>
        <div>
          <dt className="mb-0.5 text-xs text-muted-foreground">Evidence</dt>
          <dd className="flex flex-wrap gap-x-4 gap-y-1">
            <Link
              to={`/run/${encodeURIComponent(run.run_id)}/diff`}
              className="text-primary hover:underline"
            >
              compare to baseline
            </Link>
            <a
              href={traceUrl(run.run_id)}
              target="_blank"
              rel="noreferrer"
              className="text-primary hover:underline"
            >
              raw trace.jsonl
            </a>
          </dd>
        </div>
      </dl>

      {inputs.length > 0 && (
        <details className="min-w-0 max-w-full overflow-hidden border-y text-sm">
          <summary className="select-none py-2.5 font-medium hover:text-foreground">
            Run configuration
            <span className="ml-2 text-xs font-normal text-muted-foreground">
              {inputs.length} input{inputs.length === 1 ? "" : "s"}
            </span>
          </summary>
          <div className="min-w-0 space-y-4 border-t py-4">
            <p className="text-xs text-muted-foreground">
              Exact recorded inputs for debugging and reproduction.
            </p>
            <dl className="min-w-0 space-y-3">
              {inputs.map(([key, value]) => (
                <div key={key} className="min-w-0">
                  <dt className="break-all font-mono text-xs font-semibold">
                    {key}
                  </dt>
                  <dd className="mt-1 min-w-0">
                    <code className="block max-w-full whitespace-pre-wrap break-all rounded bg-muted px-2 py-1.5 font-mono text-xs">
                      {JSON.stringify(value)}
                    </code>
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        </details>
      )}
    </div>
  );
}
