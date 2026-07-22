// Run summary (#86): inputs, top-level verdict, criterion → check
// table. For an in-progress run (#84) the page subscribes to the SSE
// live stream and folds events into the same shape, finalizing when
// `run_finished` arrives.

import { useEffect, useState } from "react";
import { Link, useLocation, useParams } from "react-router-dom";
import {
  fetchCheck,
  fetchRun,
  liveUrl,
  traceUrl,
  type RunDetail,
  type SpanModel,
  type TraceEvent,
} from "../api";
import { foldRun } from "../fold";
import { SpanChain } from "./CheckPage";
import { VerdictBadge, formatStartedAt } from "../ui";

// #280 Phase 2/3: an Allure-style status roll-up for a run — a donut of
// the check verdicts plus a count legend. Mechanically derived from the
// recorded criterion → check verdicts, never re-judged.
export interface StatusTally {
  pass: number;
  fail: number;
  inconclusive: number;
  pending: number;
  total: number;
}

export function tallyChecks(criteria: RunDetail["criteria"]): StatusTally {
  const t: StatusTally = { pass: 0, fail: 0, inconclusive: 0, pending: 0, total: 0 };
  for (const c of criteria) {
    for (const chk of c.checks) {
      t.total++;
      const v = chk.verdict;
      if (v === "pass") t.pass += 1;
      else if (v === "fail") t.fail += 1;
      else if (v && v.startsWith("inconclusive")) t.inconclusive += 1;
      else t.pending += 1;
    }
  }
  return t;
}

const DONUT_SEGMENTS: { key: keyof Omit<StatusTally, "total">; cls: string; label: string }[] = [
  { key: "pass", cls: "seg-pass", label: "passed" },
  { key: "fail", cls: "seg-fail", label: "failed" },
  { key: "inconclusive", cls: "seg-inconclusive", label: "inconclusive" },
  { key: "pending", cls: "seg-pending", label: "pending" },
];

// A pure-SVG donut: each status is an arc whose length is its share of
// the total, stacked by advancing the dash offset. Total in the middle.
export function StatusDonut({ tally }: { tally: StatusTally }) {
  const total = tally.total || 1;
  const r = 42;
  const circ = 2 * Math.PI * r;
  let offset = 0;
  const arcs = DONUT_SEGMENTS.map((s) => {
    const n = tally[s.key];
    if (n === 0) return null;
    const len = (n / total) * circ;
    const arc = (
      <circle
        key={s.key}
        className={`donut-seg ${s.cls}`}
        cx="50"
        cy="50"
        r={r}
        strokeDasharray={`${len} ${circ - len}`}
        strokeDashoffset={-offset}
      />
    );
    offset += len;
    return arc;
  });
  return (
    <div className="panel status-summary" data-testid="status-summary">
      <svg
        className="donut"
        viewBox="0 0 100 100"
        width="88"
        height="88"
        role="img"
        aria-label={`${tally.pass} passed, ${tally.fail} failed, ${tally.inconclusive} inconclusive of ${tally.total} checks`}
      >
        <circle className="donut-track" cx="50" cy="50" r={r} />
        {arcs}
        <text className="donut-center" x="50" y="52" textAnchor="middle">
          {tally.total}
        </text>
      </svg>
      <ul className="status-counts">
        {DONUT_SEGMENTS.map((s) => (
          <li key={s.key} className={`count ${s.cls}`} data-testid={`count-${s.key}`}>
            <span className="count-n">{tally[s.key]}</span> {s.label}
          </li>
        ))}
      </ul>
    </div>
  );
}

function useRun(runId: string): { run: RunDetail | null; error: string | null } {
  const [run, setRun] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let source: EventSource | null = null;
    let cancelled = false;
    const events: TraceEvent[] = [];

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
        const folded = foldRun(runId, events);
        setRun(folded);
        if (evt.kind === "run_finished") {
          source?.close();
          // Re-fetch the authoritative server rendering (duration,
          // verification naming) now that the run is complete.
          fetchRun(runId).then((d) => !cancelled && setRun(d), () => {});
        }
      });
      source.onerror = () => {
        // Stream closed (server cap or network); the page keeps the
        // last folded state. A reload resumes via replay.
        source?.close();
      };
    }, (e) => setError(String(e)));

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [runId]);

  return { run, error };
}

// #280 Phase 2: a run-scoped top tab bar. The tabs are separate hash
// routes (`/run/:id`, `…/suites`, `…/categories`, `…/timeline`) so each
// deep-links and survives a reload; the run's own SSE/data lifecycle
// lives once in RunPage regardless of tab.
const RUN_TABS = [
  { key: "overview", label: "Overview", suffix: "" },
  { key: "suites", label: "Suites", suffix: "/suites" },
  { key: "categories", label: "Categories", suffix: "/categories" },
  { key: "timeline", label: "Timeline", suffix: "/timeline" },
] as const;

const TAB_KEYS = ["suites", "categories", "timeline"] as const;

// The active tab is the trailing path segment; anything else (the bare
// `/run/:id`) is the Overview index.
export function activeTab(pathname: string): string {
  const last = pathname.split("/").filter(Boolean).pop() ?? "";
  return (TAB_KEYS as readonly string[]).includes(last) ? last : "overview";
}

function RunTabs({ runId, active }: { runId: string; active: string }) {
  const base = `/run/${encodeURIComponent(runId)}`;
  return (
    <nav className="run-tabs" data-testid="run-tabs">
      {RUN_TABS.map((t) => (
        <Link
          key={t.key}
          to={`${base}${t.suffix}`}
          className={`run-tab ${active === t.key ? "on" : ""}`}
          aria-current={active === t.key ? "page" : undefined}
        >
          {t.label}
        </Link>
      ))}
    </nav>
  );
}

export default function RunPage() {
  const { runId = "" } = useParams();
  const { run, error } = useRun(runId);
  const tab = activeTab(useLocation().pathname);

  if (error) return <p className="error">{error}</p>;
  if (run === null) return <p className="muted">Loading…</p>;

  const empty = run.criteria.length === 0;
  return (
    <>
      <p className="kv">
        <Link to="/">← runs</Link>
      </p>
      <h2 className="run-title">
        {run.verification} · <code>{run.run_id}</code>{" "}
        <VerdictBadge verdict={run.verdict} live={run.live} />
      </h2>
      <RunTabs runId={run.run_id} active={tab} />
      {run.setup_aborted && (
        <p className="notice">
          Setup aborted — no checks ran. The verdict reflects the abort, not the
          artifact.
        </p>
      )}
      {empty ? (
        <p className="muted">No criteria recorded{run.live ? " yet" : ""}.</p>
      ) : tab === "suites" ? (
        <SuitesTab run={run} />
      ) : tab === "categories" ? (
        <CategoriesTab run={run} />
      ) : tab === "timeline" ? (
        <TimelineTab run={run} />
      ) : (
        <OverviewTab run={run} />
      )}
    </>
  );
}

// ---- Overview tab: the status roll-up + run metadata ----------------
function OverviewTab({ run }: { run: RunDetail }) {
  return (
    <>
      <StatusDonut tally={tallyChecks(run.criteria)} />
      <p className="kv meta-line">
        started {formatStartedAt(run.started_at)} ·{" "}
        <Link to={`/run/${encodeURIComponent(run.run_id)}/diff`}>compare to baseline</Link> ·{" "}
        <a href={traceUrl(run.run_id)} target="_blank" rel="noreferrer">
          raw trace.jsonl
        </a>
      </p>
      {Object.keys(run.inputs).length > 0 && (
        <p className="kv">
          inputs:{" "}
          {Object.entries(run.inputs).map(([k, v]) => (
            <span key={k}>
              <code>
                {k}={JSON.stringify(v)}
              </code>{" "}
            </span>
          ))}
        </p>
      )}
    </>
  );
}

// ---- Suites tab: the criterion → check tree + status filter chips ---
function checkMatches(verdict: string | null, filter: string): boolean {
  switch (filter) {
    case "pass":
      return verdict === "pass";
    case "fail":
      return verdict === "fail";
    case "inconclusive":
      return !!verdict && verdict.startsWith("inconclusive");
    default:
      return true; // "all"
  }
}

function SuitesTab({ run }: { run: RunDetail }) {
  const [filter, setFilter] = useState<string>("all");
  const t = tallyChecks(run.criteria);
  const chips = [
    { key: "all", n: t.total, label: "all", cls: "" },
    { key: "pass", n: t.pass, label: "✓ pass", cls: "ok" },
    { key: "fail", n: t.fail, label: "✗ fail", cls: "bad" },
    { key: "inconclusive", n: t.inconclusive, label: "? inconclusive", cls: "inc" },
  ];
  const criteria = run.criteria
    .map((c) => ({ ...c, checks: c.checks.filter((chk) => checkMatches(chk.verdict, filter)) }))
    .filter((c) => c.checks.length > 0);
  return (
    <>
      <div className="chips" data-testid="suite-filter">
        {chips.map((c) => (
          <button
            key={c.key}
            type="button"
            className={`chip ${filter === c.key ? "on" : ""}`}
            onClick={() => setFilter(c.key)}
            data-testid={`chip-${c.key}`}
          >
            <span className={`n ${c.cls}`}>{c.n}</span> {c.label}
          </button>
        ))}
      </div>
      {criteria.length === 0 ? (
        <p className="muted">No checks match this filter.</p>
      ) : (
        <table className="runs">
          <thead>
            <tr>
              <th>criterion / check</th>
              <th>verdict</th>
            </tr>
          </thead>
          <tbody>
            {criteria.map((criterion) => (
              <CriterionRows key={criterion.id} runId={run.run_id} criterion={criterion} />
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}

// ---- Categories tab: non-passing checks grouped by mechanical cause -
// The verdict already IS the category — `fail` (the artifact is wrong)
// vs `inconclusive:<cause>` (the harness/environment couldn't decide).
// No LLM, no rules engine: the split is native to the judge's output.
function checkLink(runId: string, criterionId: string, checkId: string): string {
  return `/run/${encodeURIComponent(runId)}/check/${encodeURIComponent(
    `${criterionId}::${checkId}`,
  )}`;
}

function causeLabel(cause: string): string {
  if (cause === "fail") return "✗ fail · the artifact is wrong";
  return `? inconclusive · ${cause.replace(/^inconclusive:/, "")}`;
}

function causeBlurb(cause: string): string {
  return cause === "fail"
    ? "The delivery web behaved wrong — these are product defects."
    : "Couldn't be decided — the harness or environment, not the artifact.";
}

// One check under a cause group, lazily resolving its first non-passing
// assertion's recorded (now semantic, #280) detail.
function CategoryItem({
  runId,
  criterionId,
  checkId,
}: {
  runId: string;
  criterionId: string;
  checkId: string;
}) {
  const [detail, setDetail] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    fetchCheck(runId, criterionId, checkId).then((d) => {
      if (!live) return;
      const a = d.timeline.find((e) => e.kind === "assertion_evaluated" && e.state !== "pass");
      setDetail(a && typeof a.detail === "string" ? a.detail : null);
    }, () => {});
    return () => {
      live = false;
    };
  }, [runId, criterionId, checkId]);
  return (
    <li data-testid="category-item">
      <Link to={checkLink(runId, criterionId, checkId)}>
        <b>{checkId}</b>
      </Link>{" "}
      <span className="muted">· {criterionId}</span>
      {detail && (
        <>
          {" "}
          — <code className="cat-detail">{detail}</code>
        </>
      )}
    </li>
  );
}

function CategoriesTab({ run }: { run: RunDetail }) {
  const groups = new Map<string, { criterionId: string; checkId: string }[]>();
  for (const c of run.criteria) {
    for (const chk of c.checks) {
      const v = chk.verdict;
      if (!v || v === "pass") continue;
      const cause = v === "fail" ? "fail" : v;
      const bucket = groups.get(cause) ?? [];
      bucket.push({ criterionId: c.id, checkId: chk.id });
      groups.set(cause, bucket);
    }
  }
  if (groups.size === 0) {
    return <p className="muted">No failures — every check passed.</p>;
  }
  // `fail` first, then inconclusive causes alphabetically.
  const ordered = [...groups.entries()].sort(([a], [b]) =>
    a === "fail" ? -1 : b === "fail" ? 1 : a.localeCompare(b),
  );
  return (
    <div className="panel">
      <h2 className="panelh">
        Failures grouped by cause <span className="dim">— mechanical, from the verdict</span>
      </h2>
      {ordered.map(([cause, items]) => (
        <div className="catgroup" key={cause} data-testid={`cat-${cause}`}>
          <h3>
            {causeLabel(cause)} <span className="cnt">({items.length})</span>
          </h3>
          <p className="cat-sub">{causeBlurb(cause)}</p>
          <ul>
            {items.map((it) => (
              <CategoryItem
                key={`${it.criterionId}::${it.checkId}`}
                runId={run.run_id}
                criterionId={it.criterionId}
                checkId={it.checkId}
              />
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

// ---- Timeline tab: the delivery-web layer chain, per check ----------
// Duhem's native "timeline" is the ordered layers a check crossed
// (#192/#193), not a worker gantt. Listed per check across the run.
function CheckSpanRow({
  runId,
  criterionId,
  check,
}: {
  runId: string;
  criterionId: string;
  check: { id: string; verdict: string | null };
}) {
  const [spans, setSpans] = useState<SpanModel[] | null>(null);
  useEffect(() => {
    let live = true;
    fetchCheck(runId, criterionId, check.id).then(
      (d) => live && setSpans(d.spans),
      () => live && setSpans([]),
    );
    return () => {
      live = false;
    };
  }, [runId, criterionId, check.id]);
  return (
    <div className="span-row" data-testid="span-row">
      <div className="span-row-head">
        <Link to={checkLink(runId, criterionId, check.id)}>{check.id}</Link>{" "}
        <VerdictBadge verdict={check.verdict} />
      </div>
      {spans === null ? (
        <span className="muted">loading spans…</span>
      ) : (
        <SpanChain spans={spans} />
      )}
    </div>
  );
}

function TimelineTab({ run }: { run: RunDetail }) {
  return (
    <div className="panel">
      <h2 className="panelh">
        Delivery web per check <span className="dim">— the layers each check crossed</span>
      </h2>
      {run.criteria.flatMap((c) =>
        c.checks.map((chk) => (
          <CheckSpanRow
            key={`${c.id}::${chk.id}`}
            runId={run.run_id}
            criterionId={c.id}
            check={chk}
          />
        )),
      )}
    </div>
  );
}

function CriterionRows({
  runId,
  criterion,
}: {
  runId: string;
  criterion: RunDetail["criteria"][number];
}) {
  return (
    <>
      <tr>
        <td>
          <strong>{criterion.id}</strong>
        </td>
        <td>
          <VerdictBadge verdict={criterion.verdict} />
        </td>
      </tr>
      {criterion.checks.map((check) => (
        <CheckRows key={check.id} runId={runId} criterionId={criterion.id} check={check} />
      ))}
    </>
  );
}

// ③ failure-first (#193): a non-passing check auto-expands its
// non-passing assertions inline — the judge's recorded state plus the
// evidence-bound detail ("actual X, expected Y") — so the failure is
// legible without leaving the run page. Passing checks stay compact.
function CheckRows({
  runId,
  criterionId,
  check,
}: {
  runId: string;
  criterionId: string;
  check: { id: string; verdict: string | null };
}) {
  const failing = check.verdict !== null && check.verdict !== "pass";
  const [assertions, setAssertions] = useState<TraceEvent[]>([]);

  useEffect(() => {
    if (!failing) return;
    fetchCheck(runId, criterionId, check.id).then(
      (detail) =>
        setAssertions(
          detail.timeline.filter(
            (e) => e.kind === "assertion_evaluated" && e.state !== "pass",
          ),
        ),
      () => {},
    );
  }, [runId, criterionId, check.id, failing]);

  return (
    <>
      <tr className="nested">
        <td>
          <Link
            to={`/run/${encodeURIComponent(runId)}/check/${encodeURIComponent(
              `${criterionId}::${check.id}`,
            )}`}
          >
            {check.id}
          </Link>
        </td>
        <td>
          <VerdictBadge verdict={check.verdict} />
        </td>
      </tr>
      {assertions.map((a) => (
        <tr key={a.seq} className="nested assertion" data-testid="failing-assertion">
          <td>
            <span className="muted">assertion #{String(a.assertion_index)}</span>
            {typeof a.detail === "string" && a.detail && (
              <>
                {" "}
                <code>{a.detail}</code>
              </>
            )}
          </td>
          <td>
            <VerdictBadge verdict={String(a.state)} />
          </td>
        </tr>
      ))}
    </>
  );
}
