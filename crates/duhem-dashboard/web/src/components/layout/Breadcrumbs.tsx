import { Fragment } from "react";
import { Link, useLocation } from "react-router-dom";

import type { RunsListEntry } from "@/api";
import { useOptionalRunsData } from "@/runs-context";
import { useRunBreadcrumbDetail } from "@/run-breadcrumb";

import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

interface Crumb {
  label: string;
  to?: string;
  title?: string;
}

function findRun(runs: RunsListEntry[], runId: string): RunsListEntry | undefined {
  for (const run of runs) {
    if (run.run_id === runId) return run;
    const child = findRun(run.children ?? [], runId);
    if (child) return child;
  }
  return undefined;
}

/** Prefer the shared list, then the open run's loaded detail. */
function runCrumbLabel(
  runId: string,
  runs: RunsListEntry[] | null,
  detail: { verification: string; run_number: number } | null = null,
): string {
  const run = runs ? findRun(runs, runId) : undefined;
  const label = run ?? detail;
  return label ? `${label.verification} #${label.run_number}` : "Run";
}

// Derive breadcrumbs from the hash-router path. Kept exported for tests.
export function crumbsFor(
  pathname: string,
  runs: RunsListEntry[] | null = null,
  detail: { verification: string; run_number: number } | null = null,
): Crumb[] {
  const seg = pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (seg.length === 0) return [{ label: "Overview" }];
  const [a, b, c, d] = seg;
  if (a === "runs") return [{ label: "Runs" }];
  if (a === "verifications") return [{ label: "Verifications" }];
  if (a === "verification") {
    return [
      { label: "Verifications", to: "/verifications" },
      { label: b ?? "" },
    ];
  }
  if (a === "run" && b) {
    const runsCrumb: Crumb = { label: "Runs", to: "/runs" };
    const run: Crumb = {
      label: runCrumbLabel(b, runs, detail),
      title: b,
      to: `/run/${encodeURIComponent(b)}`,
    };
    if (c === "check") {
      // The pair is `criterion::check`; show just the check id as the
      // leaf — the run crumb already carries the run, and the criterion
      // is visible in the run's tree rail.
      const checkId = (d ?? "").split("::").pop() ?? d ?? "";
      return [runsCrumb, run, { label: checkId, title: d }];
    }
    if (c === "criterion") {
      return [runsCrumb, run, { label: d ?? "", title: d }];
    }
    if (c === "diff") return [runsCrumb, run, { label: "diff" }];
    return [runsCrumb, { label: runCrumbLabel(b, runs, detail), title: b }];
  }
  return [{ label: "Overview" }];
}

export function Breadcrumbs() {
  const runs = useOptionalRunsData()?.runs ?? null;
  const pathname = useLocation().pathname;
  const runId = pathname.match(/^\/run\/([^/]+)/)?.[1];
  const detail = useRunBreadcrumbDetail(runId ? decodeURIComponent(runId) : "");
  const crumbs = crumbsFor(pathname, runs, detail);
  return (
    <Breadcrumb>
      <BreadcrumbList>
        {crumbs.map((crumb, i) => {
          const last = i === crumbs.length - 1;
          return (
            <Fragment key={`${crumb.label}-${i}`}>
              <BreadcrumbItem>
                {crumb.to && !last ? (
                  <BreadcrumbLink asChild className="text-muted-foreground">
                    <Link to={crumb.to} title={crumb.title}>
                      {crumb.label}
                    </Link>
                  </BreadcrumbLink>
                ) : (
                  <BreadcrumbPage title={crumb.title}>
                    {crumb.label}
                  </BreadcrumbPage>
                )}
              </BreadcrumbItem>
              {!last && <BreadcrumbSeparator />}
            </Fragment>
          );
        })}
      </BreadcrumbList>
    </Breadcrumb>
  );
}
