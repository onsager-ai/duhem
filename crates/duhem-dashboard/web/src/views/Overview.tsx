import type { ComponentType } from "react";
import { CircleCheckBig, Inbox, ListChecks, Radio, TriangleAlert } from "lucide-react";
import { Link } from "react-router-dom";

import { PageHeader } from "@/components/layout/PageHeader";
import { CardsSkeleton, EmptyState, ErrorState } from "@/components/states";
import { VerdictTrend } from "@/components/VerdictTrend";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { useRunsData } from "@/runs-context";
import { computeStats } from "@/stats";
import { formatStartedAt, VerdictBadge } from "@/ui";

function StatCard({
  label,
  value,
  hint,
  tone,
  icon: Icon,
}: {
  label: string;
  value: string | number;
  hint?: string;
  tone?: string;
  icon: ComponentType<{ className?: string }>;
}) {
  return (
    <Card>
      <CardContent className="flex flex-col gap-1">
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium text-muted-foreground">
            {label}
          </span>
          <Icon className="size-4 text-muted-foreground/70" />
        </div>
        <div className={cn("text-3xl font-semibold tabular-nums", tone)}>
          {value}
        </div>
        {hint && <div className="text-xs text-muted-foreground">{hint}</div>}
      </CardContent>
    </Card>
  );
}

export default function Overview() {
  const { runs, error } = useRunsData();

  if (error) return <ErrorState error={error} />;

  if (!runs) {
    return (
      <div className="space-y-6">
        <PageHeader title="Overview" />
        <CardsSkeleton />
        <Skeleton className="h-56 rounded-xl" />
      </div>
    );
  }

  const s = computeStats(runs);

  if (s.total === 0) {
    return (
      <div className="space-y-6">
        <PageHeader title="Overview" />
        <EmptyState
          icon={Inbox}
          title="No runs yet"
          hint="Run a verification to see health, trends, and evidence here."
        />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Overview"
        description="Verification health across all recorded runs."
        actions={
          <Button asChild variant="outline" size="sm">
            <Link to="/runs">View all runs</Link>
          </Button>
        }
      />

      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <StatCard
          label="Pass rate"
          value={s.passRate === null ? "—" : `${Math.round(s.passRate * 100)}%`}
          hint={`${s.pass}/${s.pass + s.fail + s.inconclusive} decided`}
          tone="text-pass"
          icon={CircleCheckBig}
        />
        <StatCard
          label="Failing verifications"
          value={s.failingVerifications}
          hint={`of ${s.verifications} total`}
          tone={s.failingVerifications > 0 ? "text-fail" : undefined}
          icon={TriangleAlert}
        />
        <StatCard
          label="Live runs"
          value={s.live}
          hint="in progress"
          tone={s.live > 0 ? "text-live" : undefined}
          icon={Radio}
        />
        <StatCard
          label="Total runs"
          value={s.total}
          hint={`${s.verifications} verifications`}
          icon={ListChecks}
        />
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-1">
          <CardContent className="flex flex-col gap-4">
            <div className="text-sm font-medium">Recent trend</div>
            <VerdictTrend trend={s.trend} />
            <dl className="mt-1 space-y-1.5 text-sm">
              <TrendRow tone="bg-pass" label="Pass" value={s.pass} />
              <TrendRow tone="bg-fail" label="Fail" value={s.fail} />
              <TrendRow
                tone="bg-inconclusive"
                label="Inconclusive"
                value={s.inconclusive}
              />
            </dl>
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
          <CardContent className="flex flex-col gap-1">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-sm font-medium">Recent runs</span>
              <Link
                to="/runs"
                className="text-xs text-muted-foreground hover:text-foreground"
              >
                all runs →
              </Link>
            </div>
            <ul className="divide-y">
              {s.recent.map((r) => (
                <li
                  key={r.run_id}
                  className="flex items-center gap-3 py-2 text-sm"
                >
                  <Link
                    to={`/run/${encodeURIComponent(r.run_id)}`}
                    className="truncate font-mono text-xs text-foreground hover:underline"
                  >
                    {r.run_id}
                  </Link>
                  <Link
                    to={`/verification/${encodeURIComponent(r.verification)}`}
                    className="hidden truncate text-muted-foreground hover:text-foreground sm:block"
                  >
                    {r.verification}
                  </Link>
                  <span className="ml-auto hidden shrink-0 text-xs text-muted-foreground md:block">
                    {formatStartedAt(r.started_at)}
                  </span>
                  <VerdictBadge verdict={r.verdict} live={r.live} />
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function TrendRow({
  tone,
  label,
  value,
}: {
  tone: string;
  label: string;
  value: number;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className={cn("size-2.5 rounded-full", tone)} />
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="ml-auto font-medium tabular-nums">{value}</dd>
    </div>
  );
}
