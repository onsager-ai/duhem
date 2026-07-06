// ② VD-over-time (#193): one verification's criteria as a stable
// spine, each criterion's verdict tracked across runs (newest first).
// Makes "the criterion held for N runs while its checks churned"
// visible — criteria are the human commitment, checks are derivative.
// Every verdict shown is the judge's recorded verdict from the store.

import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  fetchRun,
  fetchVerificationHistory,
  type RunDetail,
  type VerificationHistory,
} from "../api";
import { VerdictBadge, formatStartedAt, verdictClass } from "../ui";

export function Sparkline({ verdicts }: { verdicts: (string | null)[] }) {
  return (
    <span className="sparkline" data-testid="sparkline">
      {verdicts.map((v, i) => (
        <span
          key={i}
          className={`dot ${v === null ? "dot-absent" : verdictClass(v)}`}
          title={v ?? "not on this run"}
        />
      ))}
    </span>
  );
}

export default function VerificationPage() {
  const { name = "" } = useParams();
  const [history, setHistory] = useState<VerificationHistory | null>(null);
  const [latest, setLatest] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetchVerificationHistory(name).then(
      (h) => {
        setHistory(h);
        // Derivative-check annotation: the latest run names each
        // criterion's current checks.
        const latestRun = h.runs[0];
        if (latestRun) {
          fetchRun(latestRun.run_id).then(setLatest, () => {});
        }
      },
      (e) => setError(String(e)),
    );
  }, [name]);

  if (error) return <p className="error">{error}</p>;
  if (history === null) return <p className="muted">Loading…</p>;

  const checksOf = (criterionId: string): string[] =>
    latest?.criteria.find((c) => c.id === criterionId)?.checks.map((c) => c.id) ?? [];

  return (
    <>
      <p className="kv">
        <Link to="/">← runs</Link>
      </p>
      <div className="panel">
        <h2>{history.name} · history</h2>
        <p className="kv">
          {history.runs.length} run{history.runs.length === 1 ? "" : "s"}, newest
          first · latest {formatStartedAt(history.runs[0]?.started_at ?? null)}{" "}
          <VerdictBadge verdict={history.runs[0]?.verdict ?? null} />
        </p>
      </div>
      <table className="runs">
        <thead>
          <tr>
            <th>criterion</th>
            <th>history (newest → oldest)</th>
          </tr>
        </thead>
        <tbody>
          {history.criteria.map((criterion) => (
            <tr key={criterion.criterion_id}>
              <td>
                <strong>{criterion.criterion_id}</strong>
                {checksOf(criterion.criterion_id).length > 0 && (
                  <div className="muted checks-note">
                    checks: {checksOf(criterion.criterion_id).join(", ")}
                  </div>
                )}
              </td>
              <td>
                <Sparkline verdicts={criterion.verdicts} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="panel">
        <h2>Runs</h2>
        <table className="runs">
          <thead>
            <tr>
              <th>run</th>
              <th>started</th>
              <th>verdict</th>
            </tr>
          </thead>
          <tbody>
            {history.runs.map((run) => (
              <tr key={run.run_id}>
                <td>
                  <Link to={`/run/${encodeURIComponent(run.run_id)}`}>{run.run_id}</Link>
                </td>
                <td>{formatStartedAt(run.started_at)}</td>
                <td>
                  <VerdictBadge verdict={run.verdict} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
