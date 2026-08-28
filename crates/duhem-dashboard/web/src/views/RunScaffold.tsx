// The run report's shared workspace (#323): compact sticky run context,
// horizontal Summary / Results / Definition tabs, and a flat split view
// inside Results. The criteria → checks rail is deliberately scoped to
// Results; Summary and Definition get the full content width.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { ChevronRight, FileText, ListChecks, LayoutList } from "lucide-react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";

import {
  fetchCheck,
  fetchRun,
  liveUrl,
  type CheckDetail,
  type RunDetail,
  type TraceEvent,
} from "../api";
import { foldRun } from "../fold";
import { deliveryLayerLabel, groupTimeline, stepStatus } from "../format";
import { flowOrigin } from "../definition";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  buildSuiteTree,
  countSuiteStatuses,
  filterSuiteTree,
  SUITE_STATUSES,
  type SuiteCriterionNode,
  type SuiteStatus,
} from "../suite-tree";
import { StatusBadge, VerdictBadge } from "../ui";
import { DefinitionProvider, useVd } from "./definition-context";

export type ConnectionState = "connected" | "reconnecting" | "disconnected";

// Subscribe to a run: lifecycle comes from recorded detail. Connection
// is viewer-local SSE state and never enters the API/static-export shape.
export function useRun(runId: string): {
  run: RunDetail | null;
  error: string | null;
  connection: ConnectionState;
} {
  const [run, setRun] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connection, setConnection] =
    useState<ConnectionState>("disconnected");

  useEffect(() => {
    let source: EventSource | null = null;
    let cancelled = false;
    const events: TraceEvent[] = [];

    setRun(null);
    setError(null);
    setConnection("disconnected");
    fetchRun(runId).then((detail) => {
      if (cancelled) return;
      setRun(detail);
      if (detail.status !== "running") return;
      // Replay-then-follow: the SSE stream re-sends the whole trace,
      // so folding from scratch is gap- and dupe-free by contract.
      setConnection("reconnecting");
      source = new EventSource(liveUrl(runId));
      source.onopen = () => !cancelled && setConnection("connected");
      source.addEventListener("trace", (msg) => {
        const evt = JSON.parse((msg as MessageEvent).data) as TraceEvent;
        events.push(evt);
        setRun(foldRun(runId, events));
        if (evt.kind === "run_finished" || evt.kind === "run_aborted") {
          source?.close();
          setConnection("disconnected");
          fetchRun(runId).then((d) => !cancelled && setRun(d), () => {});
        }
      });
      source.onerror = () => !cancelled && setConnection("reconnecting");
    }, (e) => setError(String(e)));

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [runId]);

  return { run, error, connection };
}

function checkHref(runId: string, criterionId: string, checkId: string): string {
  return `/run/${encodeURIComponent(runId)}/check/${encodeURIComponent(
    `${criterionId}::${checkId}`,
  )}`;
}

function resultsHref(run: RunDetail): string {
  const criterion =
    run.criteria.find((item) => item.verdict !== "pass") ??
    run.criteria[0];
  if (!criterion) return `/run/${encodeURIComponent(run.run_id)}/results`;
  const check =
    criterion.checks.find((item) => item.verdict !== "pass") ??
    criterion.checks[0];
  return check
    ? checkHref(run.run_id, criterion.id, check.id)
    : criterionHref(run.run_id, criterion.id);
}

function criterionHref(runId: string, criterionId: string): string {
  return `/run/${encodeURIComponent(runId)}/criterion/${encodeURIComponent(criterionId)}`;
}

// One criterion group in the rail: a collapse toggle + its check links.
// Criteria are expanded by default so the whole run is legible at a
// glance (and every check link is in the DOM without interaction).
function TreeGroup({
  runId,
  group,
  activePair,
  activeCriterion,
  activeStep,
}: {
  runId: string;
  group: SuiteCriterionNode;
  activePair?: string;
  activeCriterion?: string;
  activeStep?: string;
}) {
  const criterion = useMemo(
    () => ({
      ...group.criterion,
      checks: group.checks.map((node) => node.check),
    }),
    [group.checks, group.criterion],
  );
  const { contextOnly } = group;
  const hasChecks = criterion.checks.length > 0;
  const [open, setOpen] = useState(hasChecks);
  const vd = useVd();
  const [search] = useSearchParams();
  const navigate = useNavigate();
  const critDesc = vd?.criterion(criterion.id)?.description;
  const active = activeCriterion === criterion.id;
  const [activeCheck, setActiveCheck] = useState<CheckDetail | null>(null);
  useEffect(() => {
    const pair = criterion.checks.find(
      (check) => activePair === `${criterion.id}::${check.id}`,
    );
    if (!pair) {
      setActiveCheck(null);
      return;
    }
    let live = true;
    fetchCheck(runId, criterion.id, pair.id).then(
      (detail) => live && setActiveCheck(detail),
      () => live && setActiveCheck(null),
    );
    return () => {
      live = false;
    };
  }, [activePair, criterion.checks, criterion.id, runId]);
  const label = (
    <>
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
    <div className="md:-mb-14" data-filter-context={contextOnly ? "true" : undefined}>
      <div
        className="criterion-tree-parent flex min-w-0 items-start gap-0.5 md:sticky md:top-0 md:z-10 md:bg-background/95 md:backdrop-blur"
        data-testid="criterion-parent"
      >
        {hasChecks ? (
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            aria-expanded={open}
            aria-label={`${open ? "Collapse" : "Expand"} ${criterion.id}`}
            className="mt-1 flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
          >
            <ChevronRight
              className={cn(
                "size-3.5 transition-transform",
                open && "rotate-90",
              )}
            />
          </button>
        ) : (
          <span className="mt-1 size-6 shrink-0" aria-hidden="true" />
        )}
        <Link
          to={criterionHref(runId, criterion.id)}
          aria-current={active ? "page" : undefined}
          title={critDesc}
          className={cn(
            "flex min-w-0 flex-1 items-start gap-1.5 rounded-md px-2 py-1.5 text-sm font-medium transition-colors",
            active
              ? "bg-accent text-accent-foreground"
              : "text-foreground hover:bg-accent/60",
          )}
        >
          {label}
        </Link>
      </div>
      {open && (
        <div
          className="ml-8 space-y-0.5 border-l pl-3 pt-0.5"
          data-testid="check-children"
        >
          {criterion.checks.map((chk) => {
            const active = activePair === `${criterion.id}::${chk.id}`;
            const chkDesc = vd?.check(criterion.id, chk.id)?.description;
            const steps = active && activeCheck
              ? groupTimeline(activeCheck.timeline).filter((node) => node.kind === "step")
              : [];
            return (
              <div key={chk.id}>
              <Link
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
              {active && steps.length > 0 && (
                <div className="ml-3 border-l pl-2" data-testid="step-children">
                  {steps.map((node) => {
                    if (node.kind !== "step") return null;
                    const started = node.events[0];
                    const uses = typeof started.uses === "string" ? started.uses : "step";
                    const flow = flowOrigin(started.flow);
                    const authoredId = vd?.stepId(criterion.id, chk.id, node.stepIndex, flow);
                    const label =
                      vd?.stepLabel(criterion.id, chk.id, node.stepIndex, flow) ??
                      `${uses} #${node.stepIndex}`;
                    const key = authoredId ?? String(node.stepIndex);
                    const stepSearch = new URLSearchParams(search);
                    stepSearch.set("step", key);
                    const status = stepStatus(node);
                    const layer = deliveryLayerLabel(started.layer);
                    const verdict =
                      status.tone === "ok"
                        ? "pass"
                        : status.tone === "fail"
                          ? "fail"
                          : status.tone === "inconclusive"
                            ? "inconclusive:step"
                            : null;
                    const moveByKeyboard = (direction: number) => {
                      const current = steps.findIndex(
                        (candidate) => candidate.kind === "step" && candidate.stepIndex === node.stepIndex,
                      );
                      const target = steps[current + direction];
                      if (target?.kind !== "step") return;
                      const targetStarted = target.events[0];
                      const targetFlow = flowOrigin(targetStarted.flow);
                      const targetKey = vd?.stepId(
                        criterion.id,
                        chk.id,
                        target.stepIndex,
                        targetFlow,
                      ) ?? String(target.stepIndex);
                      const targetSearch = new URLSearchParams(search);
                      targetSearch.set("step", targetKey);
                      navigate({
                        pathname: checkHref(runId, criterion.id, chk.id),
                        search: `?${targetSearch.toString()}`,
                      });
                    };
                    return (
                      <Link
                        key={node.stepIndex}
                        data-flow-invocation={flow?.invocation}
                        data-flow-name={flow?.name}
                        to={{
                          pathname: checkHref(runId, criterion.id, chk.id),
                          search: `?${stepSearch.toString()}`,
                        }}
                        aria-label={label}
                        aria-current={activeStep === key ? "step" : undefined}
                        onKeyDown={(event) => {
                          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                          event.preventDefault();
                          moveByKeyboard(event.key === "ArrowDown" ? 1 : -1);
                        }}
                        className={cn(
                          "flex min-w-0 items-center gap-2 rounded px-2 py-1 text-xs",
                          activeStep === key
                            ? "bg-accent font-medium text-accent-foreground"
                            : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                        )}
                      >
                        <span className="min-w-0 flex-1 truncate">{label}</span>
                        {layer && (
                          <Badge
                            variant="outline"
                            className="px-1.5 font-mono text-[0.65rem]"
                            data-testid="step-layer"
                          >
                            {layer}
                          </Badge>
                        )}
                        <VerdictBadge verdict={verdict} compact />
                      </Link>
                    );
                  })}
                </div>
              )}
              </div>
            );
          })}
        </div>
      )}
      {/* Preserve the sticky runway the filter row supplied before it moved
          outside the scroller; the parent margin cancels visible spacing. */}
      <div className="criterion-sticky-runway hidden h-14 md:block" aria-hidden="true" />
    </div>
  );
}

// The Results-only tree rail. Top-level tabs own Summary and Definition,
// so this nav can express one hierarchy clearly: criteria → checks.
function RunTree({
  run,
  activePair,
  activeCriterion,
  activeStep,
}: {
  run: RunDetail;
  activePair?: string;
  activeCriterion?: string;
  activeStep?: string;
}) {
  const [activeStatuses, setActiveStatuses] = useState<Set<SuiteStatus>>(
    () => new Set(),
  );
  const tree = useMemo(() => buildSuiteTree(run.criteria), [run.criteria]);
  const counts = useMemo(() => countSuiteStatuses(tree), [tree]);
  const filtered = useMemo(
    () => filterSuiteTree(tree, activeStatuses),
    [activeStatuses, tree],
  );
  const toggleStatus = (status: SuiteStatus) => {
    setActiveStatuses((current) => {
      const next = new Set(current);
      if (next.has(status)) next.delete(status);
      else next.add(status);
      return next;
    });
  };
  return (
    <>
      <div
        role="group"
        aria-label="Filter suites by status"
        data-testid="suite-filter"
        className="flex flex-wrap gap-1.5 border-b py-2 pr-3"
      >
        {SUITE_STATUSES.map((status) => {
          const active = activeStatuses.has(status);
          return (
            <Button
              key={status}
              type="button"
              size="sm"
              variant={active ? "secondary" : "outline"}
              className="h-7 rounded-full px-2.5 capitalize"
              aria-pressed={active}
              aria-label={`${status}, ${counts[status]} check${counts[status] === 1 ? "" : "s"}`}
              data-testid={`suite-filter-${status}`}
              onClick={() => toggleStatus(status)}
            >
              {status}
              <Badge variant={status} className="min-w-5 px-1 tabular-nums">
                {counts[status]}
              </Badge>
            </Button>
          );
        })}
      </div>
      <nav
        aria-label="criteria and checks"
        data-testid="run-tree"
        className="run-results-rail run-results-nav min-w-0 max-w-full space-y-0.5 overflow-x-hidden pb-6 pr-3 supports-[overflow:clip]:overflow-x-clip md:min-h-0 md:overflow-y-auto"
      >
        {filtered.map((group) => (
          <TreeGroup
            key={group.criterion.id}
            runId={run.run_id}
            group={group}
            activePair={activePair}
            activeCriterion={activeCriterion}
            activeStep={activeStep}
          />
        ))}
        {filtered.length === 0 && (
          <p className="px-2 py-3 text-xs text-muted-foreground">
            No suite nodes match the selected statuses.
          </p>
        )}
      </nav>
    </>
  );
}

type RunTab = "summary" | "results" | "definition";

function RunTabs({ run, active }: { run: RunDetail; active: RunTab }) {
  const tabs = [
    {
      id: "summary" as const,
      label: "Summary",
      icon: LayoutList,
      to: `/run/${encodeURIComponent(run.run_id)}`,
      enabled: true,
    },
    {
      id: "results" as const,
      label: "Results",
      icon: ListChecks,
      to: resultsHref(run),
      enabled: run.criteria.length > 0,
    },
    {
      id: "definition" as const,
      label: "Definition",
      icon: FileText,
      to: `/run/${encodeURIComponent(run.run_id)}/definition`,
      enabled: run.has_definition,
    },
  ];
  return (
    <nav
      aria-label="run detail views"
      className="run-tabs flex min-w-0 gap-5"
      role="tablist"
    >
      {tabs.map((tab) => {
        const Icon = tab.icon;
        const selected = active === tab.id;
        return tab.enabled ? (
          <Link
            key={tab.id}
            to={tab.to}
            role="tab"
            aria-selected={selected}
            aria-current={selected ? "page" : undefined}
            className={cn(
              "relative inline-flex h-9 items-center gap-1.5 whitespace-nowrap border-b-2 px-0.5 text-sm font-medium transition-colors",
              selected
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon className="size-3.5" aria-hidden="true" />
            {tab.label}
          </Link>
        ) : (
          <span
            key={tab.id}
            role="tab"
            aria-selected="false"
            aria-disabled="true"
            tabIndex={0}
            className="inline-flex h-9 items-center gap-1.5 border-b-2 border-transparent px-0.5 text-sm text-muted-foreground/50"
          >
            <Icon className="size-3.5" aria-hidden="true" />
            {tab.label}
          </span>
        );
      })}
    </nav>
  );
}

// The run report frame. `children` is the detail panel, given the
// resolved run so a page can derive from it (RunPage's donut, inputs).
export function RunScaffold({
  runId,
  activePair,
  activeCriterion,
  activeStep,
  activeDefinition,
  activeResults,
  children,
}: {
  runId: string;
  activePair?: string;
  activeCriterion?: string;
  activeStep?: string;
  activeDefinition?: boolean;
  activeResults?: boolean;
  children: (run: RunDetail) => ReactNode;
}) {
  const { run, error, connection } = useRun(runId);

  if (error) return <p className="error">{error}</p>;
  if (run === null) return <p className="text-sm text-muted-foreground">Loading…</p>;

  const active: RunTab = activeDefinition
    ? "definition"
    : activeResults || activePair || activeCriterion
      ? "results"
      : "summary";
  return (
    <DefinitionProvider runId={runId} enabled={run.has_definition}>
      <div className="run-workspace -my-6 min-w-0 max-w-full md:-my-8">
        <header className="run-workspace-header sticky top-14 z-30 -mx-4 mb-2 border-b bg-background/95 px-4 pt-2 backdrop-blur md:-mx-8 md:px-8">
          <h2 className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-base font-semibold tracking-tight">
            <span className="min-w-0 truncate">
              {run.verification}
            </span>
            <code
              className="max-w-[22ch] truncate font-mono text-xs font-normal text-muted-foreground"
              title={run.run_id}
            >
              {run.run_id}
            </code>
            <StatusBadge status={run.status} />
            <VerdictBadge verdict={run.verdict} />
            {run.status === "running" && (
              <span className="text-xs font-normal text-muted-foreground">
                {connection}
              </span>
            )}
          </h2>
          <RunTabs run={run} active={active} />
        </header>

        {active === "results" ? (
          run.criteria.length === 0 ? (
            <p className="py-6 text-sm text-muted-foreground">
              No criteria recorded{run.status === "running" ? " yet" : ""}.
            </p>
          ) : (
          <div className="run-results-grid grid min-w-0 max-w-full md:grid-cols-[17rem_minmax(0,1fr)]">
            <aside className="min-w-0 max-w-full border-b md:grid md:max-h-[calc(100vh-10.5rem)] md:grid-rows-[auto_minmax(0,1fr)] md:border-b-0 md:border-r">
              <RunTree
                run={run}
                activePair={activePair}
                activeCriterion={activeCriterion}
                activeStep={activeStep}
              />
            </aside>
            <section className="run-results-detail min-w-0 py-3 md:max-h-[calc(100vh-10.5rem)] md:overflow-y-auto md:py-0 md:pl-4">
              {children(run)}
            </section>
          </div>
          )
        ) : (
          <section className="min-w-0">{children(run)}</section>
        )}
      </div>
    </DefinitionProvider>
  );
}
