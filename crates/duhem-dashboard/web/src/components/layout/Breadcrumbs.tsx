import { Fragment } from "react";
import { Link, useLocation } from "react-router-dom";

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

// Derive breadcrumbs from the hash-router path. Kept exported for tests.
export function crumbsFor(pathname: string): Crumb[] {
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
    const runs: Crumb = { label: "Runs", to: "/runs" };
    const run: Crumb = {
      label: truncId(b),
      title: b,
      to: `/run/${encodeURIComponent(b)}`,
    };
    if (c === "check") {
      // The pair is `criterion::check`; show just the check id as the
      // leaf — the run crumb already carries the run, and the criterion
      // is visible in the run's tree rail.
      const checkId = (d ?? "").split("::").pop() ?? d ?? "";
      return [runs, run, { label: checkId, title: d }];
    }
    if (c === "criterion") {
      return [runs, run, { label: d ?? "", title: d }];
    }
    if (c === "diff") return [runs, run, { label: "diff" }];
    return [runs, { label: truncId(b), title: b }];
  }
  return [{ label: "Overview" }];
}

export function Breadcrumbs() {
  const crumbs = crumbsFor(useLocation().pathname);
  return (
    <Breadcrumb>
      <BreadcrumbList>
        {crumbs.map((crumb, i) => {
          const last = i === crumbs.length - 1;
          return (
            <Fragment key={`${crumb.label}-${i}`}>
              <BreadcrumbItem>
                {crumb.to && !last ? (
                  <BreadcrumbLink asChild>
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
