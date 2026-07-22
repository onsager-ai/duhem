// Run summary (#86): inputs, top-level verdict, criterion → check
// table. For an in-progress run (#84) the page subscribes to the SSE
// live stream and folds events into the same shape, finalizing when
// `run_finished` arrives.

import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  fetchCheck,
  fetchRun,
  liveUrl,
  traceUrl,
  type RunDetail,
  type TraceEvent,
} from "../api";
import { foldRun } from "../fold";
import { VerdictBadge, formatStartedAt } from "../ui";

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

export function tallyChecks(criteria: RunDetail["criteria"]): StatusTally {
  const t: StatusTally = { pass: 0, fail: 0, inconclusive: 0, pending: 0, total: 0 };
  for (const c of criteria) {
    for (const chk of c.checks) {
      t.total++;
      const v = chk.verdict;
      if (v === "pass") t.pass += 1;
      else if (v === "fail") t.fail += 1;
      else if (v && v.startsWith("inconclusive")) t.inconclusive += 1;
      else t.pending += 1;
    }
  }
  return t;
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
    <div className="panel status-summary" data-testid="status-summary">
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

function useRun(runId: string): { run: RunDetail | null; error: string | null } {
  const [run, setRun] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let source: EventSource | null = null;
    let cancelled = false;
    const events: TraceEvent[] = [];

    fetchRun(runId).then((detail) => {
      if (cancelled) return;
      setRun(detail);
      if (!detail.live) return;
      // Replay-then-follow: the SSE stream re-sends the whole trace,
      // so folding from scratch is gap- and dupe-free by contract.
      source = new EventSource(liveUrl(runId));
      source.addEventListener("trace", (msg) => {
        const evt = JSON.parse((msg as MessageEvent).data) as TraceEvent;
        events.push(evt);
        const folded = foldRun(runId, events);
        setRun(folded);
        if (evt.kind === "run_finished") {
          source?.close();
          // Re-fetch the authoritative server rendering (duration,
          // verification naming) now that the run is complete.
          fetchRun(runId).then((d) => !cancelled && setRun(d), () => {});
        }
      });
      source.onerror = () => {
        // Stream closed (server cap or network); the page keeps the
        // last folded state. A reload resumes via replay.
        source?.close();
      };
    }, (e) => setError(String(e)));

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [runId]);

  return { run, error };
}

export default function RunPage() {
  const { runId = "" } = useParams();
  const { run, error } = useRun(runId);

  if (error) return <p className="error">{error}</p>;
  if (run === null) return <p className="muted">Loading…</p>;

  return (
    <>
      <p className="kv">
        <Link to="/">← runs</Link>
      </p>
      <div className="panel">
        <h2>
          {run.verification} · <code>{run.run_id}</code>{" "}
          <VerdictBadge verdict={run.verdict} live={run.live} />
        </h2>
        <p className="kv">
          started {formatStartedAt(run.started_at)} ·{" "}
          <Link to={`/run/${encodeURIComponent(run.run_id)}/diff`}>compare to baseline</Link> ·{" "}
          <a href={traceUrl(run.run_id)} target="_blank" rel="noreferrer">
            raw trace.jsonl
          </a>
        </p>
        {Object.keys(run.inputs).length > 0 && (
          <p className="kv">
            inputs:{" "}
            {Object.entries(run.inputs).map(([k, v]) => (
              <span key={k}>
                <code>
                  {k}={JSON.stringify(v)}
                </code>{" "}
              </span>
            ))}
          </p>
        )}
      </div>
      {run.setup_aborted && (
        <p className="notice">
          Setup aborted — no checks ran. The verdict reflects the abort, not the
          artifact.
        </p>
      )}
      {run.criteria.length > 0 && <StatusDonut tally={tallyChecks(run.criteria)} />}
      {run.criteria.length === 0 ? (
        <p className="muted">No criteria recorded{run.live ? " yet" : ""}.</p>
      ) : (
        <table className="runs">
          <thead>
            <tr>
              <th>criterion / check</th>
              <th>verdict</th>
            </tr>
          </thead>
          <tbody>
            {run.criteria.map((criterion) => (
              <CriterionRows key={criterion.id} runId={run.run_id} criterion={criterion} />
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}

function CriterionRows({
  runId,
  criterion,
}: {
  runId: string;
  criterion: RunDetail["criteria"][number];
}) {
  return (
    <>
      <tr>
        <td>
          <strong>{criterion.id}</strong>
        </td>
        <td>
          <VerdictBadge verdict={criterion.verdict} />
        </td>
      </tr>
      {criterion.checks.map((check) => (
        <CheckRows key={check.id} runId={runId} criterionId={criterion.id} check={check} />
      ))}
    </>
  );
}

// ③ failure-first (#193): a non-passing check auto-expands its
// non-passing assertions inline — the judge's recorded state plus the
// evidence-bound detail ("actual X, expected Y") — so the failure is
// legible without leaving the run page. Passing checks stay compact.
function CheckRows({
  runId,
  criterionId,
  check,
}: {
  runId: string;
  criterionId: string;
  check: { id: string; verdict: string | null };
}) {
  const failing = check.verdict !== null && check.verdict !== "pass";
  const [assertions, setAssertions] = useState<TraceEvent[]>([]);

  useEffect(() => {
    if (!failing) return;
    fetchCheck(runId, criterionId, check.id).then(
      (detail) =>
        setAssertions(
          detail.timeline.filter(
            (e) => e.kind === "assertion_evaluated" && e.state !== "pass",
          ),
        ),
      () => {},
    );
  }, [runId, criterionId, check.id, failing]);

  return (
    <>
      <tr className="nested">
        <td>
          <Link
            to={`/run/${encodeURIComponent(runId)}/check/${encodeURIComponent(
              `${criterionId}::${check.id}`,
            )}`}
          >
            {check.id}
          </Link>
        </td>
        <td>
          <VerdictBadge verdict={check.verdict} />
        </td>
      </tr>
      {assertions.map((a) => (
        <tr key={a.seq} className="nested assertion" data-testid="failing-assertion">
          <td>
            <span className="muted">assertion #{String(a.assertion_index)}</span>
            {typeof a.detail === "string" && a.detail && (
              <>
                {" "}
                <code>{a.detail}</code>
              </>
            )}
          </td>
          <td>
            <VerdictBadge verdict={String(a.state)} />
          </td>
        </tr>
      ))}
    </>
  );
}
