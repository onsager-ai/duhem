import { ArrowRight } from "lucide-react";
import { Link, useParams } from "react-router-dom";

import type { RunDetail } from "../api";
import { VerdictBadge } from "../ui";
import { RunScaffold } from "./RunScaffold";
import { useVd } from "./definition-context";

function causeLabel(verdict: string | null): string {
  if (!verdict?.startsWith("inconclusive:")) return "No executable result was recorded.";
  const cause = verdict.slice("inconclusive:".length).replaceAll("_", " ");
  return `Duhem recorded this criterion as inconclusive: ${cause}.`;
}

function CriterionEvidence({
  run,
  criterionId,
}: {
  run: RunDetail;
  criterionId: string;
}) {
  const criterion = run.criteria.find((item) => item.id === criterionId);
  const vd = useVd();
  if (!criterion) return <p className="error">Criterion not found: {criterionId}</p>;
  const description = vd?.criterion(criterion.id)?.description;

  return (
    <div className="panel criterion-detail" data-testid="criterion-detail">
      <h2>
        {criterion.id} <VerdictBadge verdict={criterion.verdict} />
      </h2>
      {description && <p className="check-intent">{description}</p>}
      {criterion.checks.length === 0 ? (
        <div className="criterion-empty">
          <p className="criterion-empty-title">No checks were recorded</p>
          <p>
            This criterion remains navigable so the absence is visible and explainable.{" "}
            {causeLabel(criterion.verdict)}
          </p>
        </div>
      ) : (
        <div className="criterion-checks">
          <p className="kv">
            {criterion.checks.length} check{criterion.checks.length === 1 ? "" : "s"} contributed
            to this criterion.
          </p>
          {criterion.checks.map((check) => (
            <Link
              key={check.id}
              to={`/run/${encodeURIComponent(run.run_id)}/check/${encodeURIComponent(
                `${criterion.id}::${check.id}`,
              )}`}
              className="criterion-check-link"
            >
              <span>{check.id}</span>
              <span className="criterion-check-end">
                <VerdictBadge verdict={check.verdict} compact />
                <ArrowRight className="size-4" aria-hidden="true" />
              </span>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

export default function CriterionPage() {
  const { runId = "", criterionId = "" } = useParams();
  return (
    <RunScaffold runId={runId} activeCriterion={criterionId}>
      {(run) => <CriterionEvidence run={run} criterionId={criterionId} />}
    </RunScaffold>
  );
}
