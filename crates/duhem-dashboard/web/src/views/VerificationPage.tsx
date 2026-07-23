// ② VD-over-time (#193): one verification's criteria as a stable
// spine, each criterion's verdict tracked across runs (newest first).
// Makes "the criterion held for N runs while its checks churned"
// visible — criteria are the human commitment, checks are derivative.
// Every verdict shown is the judge's recorded verdict from the store.

import { useEffect, useState } from "react";
import { FileText, ShieldCheck } from "lucide-react";
import { Link, useParams } from "react-router-dom";
import {
  fetchRun,
  fetchVerificationHistory,
  type RunDetail,
  type VerificationHistory,
} from "../api";
import { VerdictBadge, formatStartedAt, verdictClass } from "../ui";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

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
  if (history === null)
    return <p className="text-sm text-muted-foreground">Loading…</p>;

  const checksOf = (criterionId: string): string[] =>
    latest?.criteria.find((c) => c.id === criterionId)?.checks.map((c) => c.id) ?? [];

  const runs = history.runs;
  const latestRun = runs[0] ?? null;

  return (
    <div className="space-y-6">
      <header className="border-b pb-4">
        <div className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          <ShieldCheck className="size-3.5" /> Verification
        </div>
        <h2 className="mt-1 text-xl font-semibold tracking-tight">
          {history.name}
        </h2>
        <p className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
          <span>
            {runs.length} run{runs.length === 1 ? "" : "s"}, newest first
          </span>
          <span aria-hidden>·</span>
          <span>latest {formatStartedAt(latestRun?.started_at ?? null)}</span>
          <VerdictBadge verdict={latestRun?.verdict ?? null} />
        </p>
        {/* #302: the current definition = the latest run's recorded snapshot. */}
        {latest?.has_definition && latestRun && (
          <Link
            to={`/run/${encodeURIComponent(latestRun.run_id)}/definition`}
            className="mt-2 inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
          >
            <FileText className="size-3.5" /> View current definition
          </Link>
        )}
      </header>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Criteria over time</CardTitle>
          <p className="text-sm text-muted-foreground">
            Each criterion is a stable commitment; its verdict is tracked across
            runs (newest → oldest) while the checks beneath it may churn.
          </p>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Criterion</TableHead>
                <TableHead>History (newest → oldest)</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {history.criteria.map((criterion) => (
                <TableRow key={criterion.criterion_id}>
                  <TableCell className="align-top">
                    <div className="font-medium">{criterion.criterion_id}</div>
                    {checksOf(criterion.criterion_id).length > 0 && (
                      <div className="mt-0.5 text-xs text-muted-foreground">
                        checks: {checksOf(criterion.criterion_id).join(", ")}
                      </div>
                    )}
                  </TableCell>
                  <TableCell className="align-top">
                    <Sparkline verdicts={criterion.verdicts} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Runs</CardTitle>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Run</TableHead>
                <TableHead>Started</TableHead>
                <TableHead>Verdict</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {runs.map((run) => (
                <TableRow key={run.run_id}>
                  <TableCell>
                    <Link
                      to={`/run/${encodeURIComponent(run.run_id)}`}
                      className="font-mono text-sm text-primary hover:underline"
                    >
                      {run.run_id}
                    </Link>
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatStartedAt(run.started_at)}
                  </TableCell>
                  <TableCell>
                    <VerdictBadge verdict={run.verdict} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
