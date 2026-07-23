// Runs list (#86): verification / verdict / date filters held in URL
// state (bookmarkable), run-set rows expanding to their leaves (#49),
// "● live" badges on in-progress runs (#84), list kept fresh by
// visibility-aware polling (#298) so in-flight runs appear and
// verdicts resolve without a manual reload.

import { useMemo } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { type RunsListEntry } from "../api";
import { usePolledRuns } from "../hooks/use-polled-runs";
import { VerdictBadge, formatDuration, formatStartedAt } from "../ui";

const VERDICT_CHIPS = ["pass", "fail", "inconclusive", "live"] as const;

export function matchesFilters(
  entry: RunsListEntry,
  verification: string,
  verdicts: string[],
  from: string,
  to: string,
): boolean {
  if (verification && entry.verification !== verification) return false;
  if (verdicts.length > 0) {
    const v = entry.verdict;
    const hit =
      (verdicts.includes("live") && entry.live) ||
      (v !== null &&
        verdicts.some((w) => w !== "live" && (v === w || v.startsWith(`${w}:`))));
    if (!hit) return false;
  }
  if (entry.started_at) {
    const t = entry.started_at.slice(0, 10);
    if (from && t < from) return false;
    if (to && t > to) return false;
  } else if (from || to) {
    return false;
  }
  return true;
}

function Row({ entry, nested }: { entry: RunsListEntry; nested?: boolean }) {
  const name =
    entry.kind === "leaf" ? (
      <Link to={`/run/${encodeURIComponent(entry.run_id)}`}>{entry.run_id}</Link>
    ) : (
      <strong>{entry.verification}</strong>
    );
  return (
    <>
      <tr className={nested ? "nested" : undefined}>
        <td>{name}</td>
        <td>
          <Link to={`/verification/${encodeURIComponent(entry.verification)}`}>
            {entry.verification}
          </Link>
        </td>
        <td>{formatStartedAt(entry.started_at)}</td>
        <td>{formatDuration(entry.duration_ms)}</td>
        <td>
          <VerdictBadge verdict={entry.verdict} live={entry.live} />
        </td>
      </tr>
      {entry.children?.map((child) => (
        <Row key={child.run_id} entry={child} nested />
      ))}
    </>
  );
}

export default function RunsList() {
  const { runs, error } = usePolledRuns();
  const [params, setParams] = useSearchParams();

  const verification = params.get("verification") ?? "";
  const verdicts = params.getAll("verdict");
  const from = params.get("from") ?? "";
  const to = params.get("to") ?? "";

  const verifications = useMemo(
    () => [...new Set((runs ?? []).map((r) => r.verification))].sort(),
    [runs],
  );

  const update = (mutate: (p: URLSearchParams) => void) => {
    const next = new URLSearchParams(params);
    mutate(next);
    setParams(next, { replace: true });
  };

  if (error) return <p className="error">{error}</p>;
  if (runs === null) return <p className="muted">Loading…</p>;

  const visible = runs.filter((r) =>
    r.kind === "run-set"
      ? (r.children ?? []).some((c) => matchesFilters(c, verification, verdicts, from, to))
      : matchesFilters(r, verification, verdicts, from, to),
  );

  return (
    <>
      <div className="filters">
        <select
          aria-label="verification"
          value={verification}
          onChange={(e) =>
            update((p) =>
              e.target.value
                ? p.set("verification", e.target.value)
                : p.delete("verification"),
            )
          }
        >
          <option value="">all verifications</option>
          {verifications.map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
        {VERDICT_CHIPS.map((chip) => (
          <button
            key={chip}
            className={`chip ${verdicts.includes(chip) ? "on" : ""}`}
            onClick={() =>
              update((p) => {
                const current = p.getAll("verdict");
                p.delete("verdict");
                const next = current.includes(chip)
                  ? current.filter((c) => c !== chip)
                  : [...current, chip];
                next.forEach((c) => p.append("verdict", c));
              })
            }
          >
            {chip}
          </button>
        ))}
        <input
          type="date"
          aria-label="from"
          value={from}
          onChange={(e) =>
            update((p) => (e.target.value ? p.set("from", e.target.value) : p.delete("from")))
          }
        />
        <input
          type="date"
          aria-label="to"
          value={to}
          onChange={(e) =>
            update((p) => (e.target.value ? p.set("to", e.target.value) : p.delete("to")))
          }
        />
      </div>
      {visible.length === 0 ? (
        <p className="muted">No runs{runs.length > 0 ? " match the filters" : " yet"}.</p>
      ) : (
        <table className="runs">
          <thead>
            <tr>
              <th>run</th>
              <th>verification</th>
              <th>started</th>
              <th>duration</th>
              <th>verdict</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((entry) => (
              <Row key={entry.run_id} entry={entry} />
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
