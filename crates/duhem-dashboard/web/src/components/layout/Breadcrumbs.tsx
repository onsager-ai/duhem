import { Fragment } from "react";
import { Link, useLocation } from "react-router-dom";

import type { RunsListEntry } from "@/api";
import { useOptionalRunsData } from "@/runs-context";

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

function truncId(id: string): string {
  return id.length > 14 ? `${id.slice(0, 12)}…` : id;
}

function findRun(runs: RunsListEntry[], runId: string): RunsListEntry | undefined {
  for (const run of runs) {
    if (run.run_id === runId) return run;
    const child = findRun(run.children ?? [], runId);
    if (child) return child;
  }
  return undefined;
}

/** `verification #N` once the shared runs list knows the run (#694),
 *  else the truncated ULID. */
function runCrumbLabel(runId: string, runs: RunsListEntry[] | null): string {
  const run = runs ? findRun(runs, runId) : undefined;
  return run ? `${run.verification} #${run.run_number}` : truncId(runId);
}

// Derive breadcrumbs from the hash-router path. Kept exported for tests.
export function crumbsFor(pathname: string, runs: RunsListEntry[] | null = null): Crumb[] {
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
      label: runCrumbLabel(b, runs),
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
    return [runsCrumb, { label: runCrumbLabel(b, runs), title: b }];
  }
  return [{ label: "Overview" }];
}

export function Breadcrumbs() {
  const runs = useOptionalRunsData()?.runs ?? null;
  const crumbs = crumbsFor(useLocation().pathname, runs);
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
