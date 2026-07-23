import { Inbox, ShieldCheck } from "lucide-react";
import { Link } from "react-router-dom";

import { PageHeader } from "@/components/layout/PageHeader";
import { EmptyState, ErrorState } from "@/components/states";
import { VerdictTrend } from "@/components/VerdictTrend";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useRunsData } from "@/runs-context";
import { verificationSummaries } from "@/stats";
import { formatStartedAt, VerdictBadge } from "@/ui";

export default function VerificationsList() {
  const { runs, error } = useRunsData();

  if (error) return <ErrorState error={error} />;

  if (!runs) {
    return (
      <div className="space-y-6">
        <PageHeader title="Verifications" />
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 6 }, (_, i) => (
            <Skeleton key={i} className="h-36 rounded-xl" />
          ))}
        </div>
      </div>
    );
  }

  const items = verificationSummaries(runs);

  if (items.length === 0) {
    return (
      <div className="space-y-6">
        <PageHeader title="Verifications" />
        <EmptyState
          icon={Inbox}
          title="No verifications yet"
          hint="Verifications appear here once runs are recorded."
        />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Verifications"
        description={`${items.length} verifications — latest verdict and recent trend.`}
      />
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {items.map((v) => (
          <Link
            key={v.name}
            to={`/verification/${encodeURIComponent(v.name)}`}
            className="group block"
          >
            <Card className="h-full gap-0 transition-colors hover:border-primary/40 hover:bg-accent/40">
              <CardContent className="flex h-full flex-col gap-3">
                <div className="flex items-start justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <ShieldCheck className="size-4 shrink-0 text-muted-foreground" />
                    <span className="truncate font-medium group-hover:underline">
                      {v.name}
                    </span>
                  </div>
                  <VerdictBadge
                    verdict={v.latest?.verdict ?? null}
                    live={v.live}
                  />
                </div>
                <VerdictTrend trend={v.recent} />
                <div className="mt-auto flex items-center justify-between pt-1 text-xs text-muted-foreground">
                  <span>
                    {v.runs} run{v.runs === 1 ? "" : "s"}
                  </span>
                  <span>{formatStartedAt(v.latest?.started_at ?? null)}</span>
                </div>
              </CardContent>
            </Card>
          </Link>
        ))}
      </div>
    </div>
  );
}
