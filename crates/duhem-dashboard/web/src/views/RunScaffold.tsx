// The run report's shared spine: a persistent tree rail (criteria →
// checks) beside a detail panel, under one run header carrying the
// verdict. RunPage renders the run summary into the panel; CheckPage
// renders a check's evidence into it — both keep the same rail, so
// drilling from a run to one of its checks never loses the structure.
//
// This replaces the former per-run tab bar (#280 Overview / Suites /
// Categories / Timeline): the tree is the navigation, the panel is the
// content. A tree node for a check is a link whose accessible name is
// the check id — the run's self-verification VD asserts exactly that.

import { useEffect, useState, type ReactNode } from "react";
import { ChevronRight, FileText, LayoutList } from "lucide-react";
import { Link } from "react-router-dom";

import {
  fetchRun,
  liveUrl,
  type RunDetail,
  type TraceEvent,
} from "../api";
import { foldRun } from "../fold";
import { cn } from "@/lib/utils";
import { VerdictBadge } from "../ui";
import { DefinitionProvider, useVd } from "./definition-context";

// Subscribe to a run: fetch its recorded detail, and for a live run
// (#84) follow the SSE stream, folding events into the same shape and
// re-fetching the authoritative rendering once it finishes.
export function useRun(runId: string): {
  run: RunDetail | null;
  error: string | null;
} {
  const [run, setRun] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let source: EventSource | null = null;
    let cancelled = false;
    const events: TraceEvent[] = [];

    setRun(null);
    setError(null);
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
        setRun(foldRun(runId, events));
        if (evt.kind === "run_finished") {
          source?.close();
          fetchRun(runId).then((d) => !cancelled && setRun(d), () => {});
        }
      });
      source.onerror = () => source?.close();
    }, (e) => setError(String(e)));

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [runId]);

  return { run, error };
}

function checkHref(runId: string, criterionId: string, checkId: string): string {
  return `/run/${encodeURIComponent(runId)}/check/${encodeURIComponent(
    `${criterionId}::${checkId}`,
  )}`;
}

// One criterion group in the rail: a collapse toggle + its check links.
// Criteria are expanded by default so the whole run is legible at a
// glance (and every check link is in the DOM without interaction).
function TreeGroup({
  runId,
  criterion,
  activePair,
}: {
  runId: string;
  criterion: RunDetail["criteria"][number];
  activePair?: string;
}) {
  const hasChecks = criterion.checks.length > 0;
  const [open, setOpen] = useState(hasChecks);
  const vd = useVd();
  const critDesc = vd?.criterion(criterion.id)?.description;
  const label = (
    <>
      {hasChecks ? (
        <ChevronRight
          className={cn(
            "mt-0.5 size-3.5 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90",
          )}
        />
      ) : (
        <span className="size-3.5 shrink-0" aria-hidden="true" />
      )}
      <span className="min-w-0 flex-1">
        <span className="block truncate">{criterion.id}</span>
        {critDesc && (
          <span className="block truncate text-xs font-normal text-muted-foreground">
            {critDesc}
          </span>
        )}
      </span>
      <VerdictBadge
        verdict={criterion.verdict}
        compact
        className="max-w-28 truncate"
      />
    </>
  );
  return (
    <div>
      {hasChecks ? (
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          title={critDesc}
          className="flex w-full min-w-0 items-start gap-1.5 rounded-md px-2 py-1.5 text-left text-sm font-medium text-foreground transition-colors hover:bg-accent/60"
        >
          {label}
        </button>
      ) : (
        <div
          title={critDesc}
          className="flex w-full min-w-0 items-start gap-1.5 rounded-md px-2 py-1.5 text-sm font-medium text-foreground"
        >
          {label}
        </div>
      )}
      {open && (
        <div className="ml-3 space-y-0.5 border-l pl-2 pt-0.5">
          {criterion.checks.map((chk) => {
            const active = activePair === `${criterion.id}::${chk.id}`;
            const chkDesc = vd?.check(criterion.id, chk.id)?.description;
            return (
              <Link
                key={chk.id}
                to={checkHref(runId, criterion.id, chk.id)}
                // The accessible name must be exactly the check id — the
                // self-verification VD locates `role: link, name: <id>`.
                aria-label={chk.id}
                aria-current={active ? "page" : undefined}
                title={chkDesc}
                className={cn(
                  "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
                  active
                    ? "bg-accent font-medium text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                )}
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate">{chk.id}</span>
                  {chkDesc && (
                    <span className="block truncate text-xs text-muted-foreground/80">
                      {chkDesc}
                    </span>
                  )}
                </span>
                <VerdictBadge
                  verdict={chk.verdict}
                  compact
                  className="max-w-24 truncate"
                />
              </Link>
            );
          })}
        </div>
      )}
    </div>
  );
}

// The tree rail: a "Summary" root (the run overview) + one group per
// criterion. `activePair` highlights the open check, or leaves Summary
// active on the bare run page.
function RunTree({
  run,
  activePair,
  activeDefinition,
}: {
  run: RunDetail;
  activePair?: string;
  activeDefinition?: boolean;
}) {
  const summaryActive = !activePair && !activeDefinition;
  return (
    <nav
      aria-label="criteria and checks"
      data-testid="run-tree"
      className="min-w-0 max-w-full space-y-0.5 overflow-hidden rounded-lg border bg-card p-2"
    >
      <Link
        to={`/run/${encodeURIComponent(run.run_id)}`}
        aria-current={summaryActive ? "page" : undefined}
        className={cn(
          "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium transition-colors",
          summaryActive
            ? "bg-accent text-accent-foreground"
            : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
        )}
      >
        <LayoutList className="size-4 shrink-0" />
        Summary
      </Link>
      {run.criteria.map((c) => (
        <TreeGroup
          key={c.id}
          runId={run.run_id}
          criterion={c}
          activePair={activePair}
        />
      ))}
      {/* The recorded VD source snapshot (#302), when the run carries one. */}
      {run.has_definition && (
        <Link
          to={`/run/${encodeURIComponent(run.run_id)}/definition`}
          aria-current={activeDefinition ? "page" : undefined}
          className={cn(
            "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium transition-colors",
            activeDefinition
              ? "bg-accent text-accent-foreground"
              : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
          )}
        >
          <FileText className="size-4 shrink-0" />
          Definition
        </Link>
      )}
    </nav>
  );
}

// The run report frame. `children` is the detail panel, given the
// resolved run so a page can derive from it (RunPage's donut, inputs).
export function RunScaffold({
  runId,
  activePair,
  activeDefinition,
  children,
}: {
  runId: string;
  activePair?: string;
  activeDefinition?: boolean;
  children: (run: RunDetail) => ReactNode;
}) {
  const { run, error } = useRun(runId);

  if (error) return <p className="error">{error}</p>;
  if (run === null) return <p className="text-sm text-muted-foreground">Loading…</p>;

  const empty = run.criteria.length === 0;
  return (
    <DefinitionProvider runId={runId} enabled={run.has_definition}>
      <div className="min-w-0 max-w-full">
        <header className="mb-6 border-b pb-4">
          <h2 className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xl font-semibold tracking-tight">
            <span>{run.verification}</span>
            <span className="text-muted-foreground">·</span>
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-sm font-normal">
              {run.run_id}
            </code>
            <VerdictBadge verdict={run.verdict} live={run.live} />
          </h2>
        </header>

        {empty ? (
          <p className="text-sm text-muted-foreground">
            No criteria recorded{run.live ? " yet" : ""}.
          </p>
        ) : (
          <div className="grid min-w-0 max-w-full gap-6 md:grid-cols-[16rem_minmax(0,1fr)]">
            <aside className="min-w-0 max-w-full md:sticky md:top-20 md:self-start">
              <RunTree
                run={run}
                activePair={activePair}
                activeDefinition={activeDefinition}
              />
            </aside>
            <section className="min-w-0">{children(run)}</section>
          </div>
        )}
      </div>
    </DefinitionProvider>
  );
}
