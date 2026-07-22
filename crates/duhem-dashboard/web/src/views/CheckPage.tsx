// Per-check evidence (#86, #206): a plain-language summary, then the
// check's slice of the trace rendered as legible rows (icon · label ·
// detail · Δ) with the raw JSON one click away, and a rich artifacts
// panel (screenshots inline, network HAR as a request table).

import { ChevronDown, ChevronRight, Maximize2, Minimize2, X } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import { fetchCheck, type ArtifactRef, type CheckDetail, type SpanModel, type TraceEvent } from "../api";
import { VerdictBadge, isImageArtifact } from "../ui";
import { compactValue, formatEvent, groupTimeline, parseComparison, stepStatus, summarizeCheck, type TimelineNode } from "../format";
import { EventIcon } from "../components/EventIcon";
import { RunScaffold } from "./RunScaffold";
import { useVd } from "./definition-context";

// Plain-language "what happened", derived mechanically from the
// recorded timeline (never re-judged, never LLM-authored).
export function CheckSummary({ detail }: { detail: CheckDetail }) {
  const s = summarizeCheck(detail);
  const tone =
    detail.verdict === "pass" ? "ok" : detail.verdict === "fail" ? "fail" : "inconclusive";
  return (
    <div className={`check-summary tone-${tone}`} data-testid="check-summary">
      <p className="summary-headline">{s.headline}</p>
      {s.failing.length > 0 && (
        <ul className="summary-failing">
          {s.failing.map((line, i) => (
            <li key={i}>{line}</li>
          ))}
        </ul>
      )}
    </div>
  );
}

// The full detail of one assertion (#284 follow-up): *what was asserted*
// (the recorded `expr` — the human-authored rule, e.g.
// `$steps.ok.outputs.exit_code == 1`), then the observed result — a
// failed comparison's `expected`/`actual` operands as a labelled pair
// (the `actual` carries the fail accent), or the verbatim detail for a
// non-comparison outcome (an inconclusive cause, a freeform judgment).
function AssertionDetail({ expr, detail }: { expr?: string; detail: string }) {
  const cmp = parseComparison(detail);
  return (
    <span className="assert-detail">
      {expr && (
        <code className="assert-expr" data-testid="assert-expr">
          {expr}
        </code>
      )}
      {cmp ? (
        <span className="assert-cmp" data-testid="assert-cmp">
          <span className="assert-cell assert-expected">
            <span className="assert-k">expected</span>
            <code className="assert-v">{cmp.expected}</code>
          </span>
          <span className="assert-cell assert-actual">
            <span className="assert-k">actual</span>
            <code className="assert-v">{cmp.actual}</code>
          </span>
        </span>
      ) : detail ? (
        <span className="assert-reason" data-testid="assert-reason">
          {detail}
        </span>
      ) : null}
    </span>
  );
}

// One trace event as a legible, self-contained row. The whole row is
// the disclosure control (#284 follow-up): a `<details>` whose
// `<summary>` is the entire icon·label·detail·time line plus a rotating
// caret, so the raw JSON is one click *anywhere on the row* away — not a
// pixel-hunt for a tiny "raw" link. A failed assertion also renders its
// expected/actual pair inline, always visible above the raw.
function TimelineRow({
  evt,
  prev,
  artifacts,
}: {
  evt: TraceEvent;
  prev: TraceEvent | undefined;
  artifacts: ArtifactRef[];
}) {
  const fe = formatEvent(evt, prev);
  const art = fe.blobSha ? artifacts.find((a) => a.id === fe.blobSha) : undefined;
  const isAssertion = evt.kind === "assertion_evaluated";
  return (
    <li
      className={`ev tone-${fe.tone}${isAssertion ? " ev-assertion" : ""}`}
      data-testid={isAssertion ? "assertion-row" : undefined}
    >
      <details className="ev-raw">
        <summary className="ev-summary">
          <span className="ev-row">
            <span className="ev-icon">
              <EventIcon name={fe.icon} />
            </span>
            <span className="ev-label">{fe.label}</span>
            <span className="ev-detail">
              {!isAssertion && fe.detail && (
                <span className="ev-detail-text" title={fe.detail}>
                  {fe.detail}
                </span>
              )}
              {art && (
                <a
                  className="ev-artifact"
                  href={art.url}
                  target="_blank"
                  rel="noreferrer"
                  onClick={(e) => e.stopPropagation()}
                >
                  open
                </a>
              )}
            </span>
            <span className="ev-time" title={evt.ts}>
              {fe.delta ?? ""}
            </span>
            <ChevronRight className="ev-caret" aria-hidden="true" />
          </span>
          {isAssertion && (fe.expr || fe.detail) && (
            <AssertionDetail expr={fe.expr} detail={fe.detail} />
          )}
        </summary>
        <pre className="ev-raw-pre">{fe.raw}</pre>
      </details>
      {/* #284 follow-up: a captured screenshot renders inline in the
          timeline (a thumbnail that expands), so the failure evidence is
          right at the step that produced it — not only in the Artifacts
          panel below. */}
      {art && isImageArtifact(art.kind, art.url) && (
        <div className="ev-shot" data-testid="ev-shot">
          <ScreenshotArtifact artifact={art} />
        </div>
      )}
    </li>
  );
}

// A step's lifecycle collapsed into one row: the action + its
// status-propagated outcome + observation count as the summary, its
// observations one click away. A judging step's implicit verdict (#280)
// is folded in via `stepStatus`, so a failed judgment paints the step
// red and surfaces its reason inline — never a green "step ok" wrapping
// a red failure. Failed steps auto-expand (Allure-style). The
// check-level signal (explicit assertions, verdict, captures) stays
// outside any group, so nothing load-bearing is hidden.
function StepGroup({
  node,
  prevOf,
  artifacts,
}: {
  node: Extract<TimelineNode, { kind: "step" }>;
  prevOf: (evt: TraceEvent) => TraceEvent | undefined;
  artifacts: ArtifactRef[];
}) {
  const started = node.events[0];
  // Scalar (non-blob) observations — the outputs the action pulled from
  // the live web (e.g. `satisfied`, `count`, `status`). Surfaced inline
  // so a step is as legible as an assertion (#284 follow-up); blob
  // captures are excluded (they render as their own rows / thumbnails).
  const scalarObs = node.events
    .filter(
      (e) =>
        (e.kind === "step_observation" || e.kind === "setup_step_observation") &&
        typeof e.blob_sha256 !== "string",
    )
    .map((e) => ({
      name: typeof e.output_name === "string" ? e.output_name : "output",
      value: e.value,
      seq: e.seq,
    }));
  const fe = formatEvent(started, prevOf(started));
  const status = stepStatus(node);
  // Overlay the authored step `id` from the recorded VD snapshot (#302):
  // a named step reads by its intent (`refund`), with the action verb
  // demoted into the detail. Anonymous steps keep the verb as the label.
  const vd = useVd();
  const cid = typeof started.criterion_id === "string" ? started.criterion_id : "";
  const chid = typeof started.check_id === "string" ? started.check_id : "";
  const stepId = vd?.stepId(cid, chid, node.stepIndex);
  const label = stepId ?? fe.label;
  const detailText = stepId ? [fe.label, fe.detail].filter(Boolean).join(" · ") : fe.detail;
  const statusObs = scalarObs.find(
    (observation) => observation.name === "status" && typeof observation.value === "number",
  );
  const httpStatus = statusObs?.value as number | undefined;
  return (
    <li className={`ev step-group tone-${status.tone}`} data-testid="step-group">
      <details open={status.failed}>
        <summary>
          {/* Primary line: what the step did + whether it ran ok. */}
          <span className="ev-row">
            <span className="ev-icon">
              <EventIcon name={fe.icon} />
            </span>
            <span className="ev-label">{label}</span>
            <span className="ev-detail">
              {detailText && (
                <span className="ev-detail-text" title={detailText}>
                  {detailText}
                </span>
              )}
              {httpStatus !== undefined && (
                <span className={`api-status ${httpStatus >= 400 ? "bad" : "ok"}`} data-testid="api-status">
                  → {httpStatus}
                </span>
              )}
              <span className={`step-outcome tone-${status.tone}`} data-testid="step-outcome">
                <EventIcon name={status.icon} />
                {status.label}
              </span>
            </span>
            <span className="ev-time" title={started.ts}>
              {fe.delta ?? ""}
            </span>
            <ChevronRight className="ev-caret" aria-hidden="true" />
          </span>
          {/* Secondary block: what it observed + (for a failed judgment)
              the reason — each on its own line, never crammed inline with
              the action text (#284 follow-up). */}
          {(scalarObs.length > 0 || status.reason) && (
            <span className="step-detail">
              {scalarObs.length > 0 && (
                <span className="step-obs" data-testid="step-obs">
                  {scalarObs.map((o) => {
                    const judged = o.name === "satisfied";
                    const tone = judged ? (o.value === true ? "ok" : "fail") : "";
                    return (
                      <span key={o.seq} className={`obs-chip${tone ? ` tone-${tone}` : ""}`}>
                        <span className="obs-k">{o.name}</span>
                        <code className="obs-v">{compactValue(o.value)}</code>
                      </span>
                    );
                  })}
                </span>
              )}
              {status.reason && (
                <span className="step-reason" data-testid="step-reason">
                  {status.reason}
                </span>
              )}
            </span>
          )}
        </summary>
        <ol className="timeline step-inner">
          {/* The full step detail — started (with its args), each
              observation, finished (with its outcome), and the folded
              implicit judgment — each row keeps its own raw toggle, so
              nothing is unreachable. */}
          {node.events.map((e) => (
            <TimelineRow key={e.seq} evt={e} prev={prevOf(e)} artifacts={artifacts} />
          ))}
        </ol>
      </details>
    </li>
  );
}

export function Timeline({
  events,
  artifacts = [],
}: {
  events: TraceEvent[];
  artifacts?: ArtifactRef[];
}) {
  const nodes = groupTimeline(events);
  const idx = new Map(events.map((e, i) => [e.seq, i]));
  const prevOf = (evt: TraceEvent) => events[(idx.get(evt.seq) ?? 0) - 1];
  return (
    <ol className="timeline">
      {nodes.map((n) =>
        n.kind === "step" ? (
          <StepGroup key={n.key} node={n} prevOf={prevOf} artifacts={artifacts} />
        ) : (
          <TimelineRow key={n.key} evt={n.event} prev={prevOf(n.event)} artifacts={artifacts} />
        ),
      )}
    </ol>
  );
}

// ④ delivery-web span chain (#193 over #192 data): the ordered layers
// the check actually crossed, colored by outcome; the first broken
// layer carries its detail. Empty spans = a pre-tag or untagged run —
// say "layer unknown", never guess.
export function SpanChain({ spans }: { spans: SpanModel[] }) {
  if (spans.length === 0) {
    return (
      <p className="kv muted" data-testid="spanchain-unknown">
        delivery web: layer unknown (run predates layer tags or steps are untagged)
      </p>
    );
  }
  return (
    <p className="spanchain" data-testid="spanchain">
      <span className="muted">delivery web: </span>
      {spans.map((s, i) => (
        <span key={s.seq}>
          {i > 0 && <ChevronRight className="span-sep" aria-hidden="true" />}
          <span
            className={`span-node ${s.ok ? "span-ok" : "span-fail"}`}
            title={`step evidence #${s.seq}${s.detail ? ` — ${s.detail}` : ""}`}
          >
            {s.layer}
            {!s.ok && <X className="span-x" aria-hidden="true" />}
            {!s.ok && s.detail ? s.detail : ""}
          </span>
        </span>
      ))}
    </p>
  );
}

function artifactLabel(kind: string): string {
  switch (kind) {
    case "capture/screenshot":
      return "Screenshot";
    case "capture/dom":
      return "DOM snapshot";
    case "capture/network":
      return "Network (HAR)";
    case "capture/target-rect":
      return "Target highlight";
    case "capture/video":
      return "Video";
    default:
      return kind;
  }
}

interface HarHeader {
  name: string;
  value: string;
}
interface HarEntry {
  request: { method: string; url: string; headers?: HarHeader[]; postData?: { text?: string } };
  response: { status: number; headers?: HarHeader[]; content?: { text?: string } };
}

function HarHeaders({ title, headers }: { title: string; headers?: HarHeader[] }) {
  if (!headers || headers.length === 0) return null;
  return (
    <div className="har-kv">
      <h4>{title}</h4>
      <dl>
        {headers.map((h, i) => (
          <Fragment key={i}>
            <dt>{h.name}</dt>
            <dd>{h.value}</dd>
          </Fragment>
        ))}
      </dl>
    </div>
  );
}

function HarBody({ title, text }: { title: string; text?: string }) {
  if (text === undefined || text === "") return null;
  return (
    <div className="har-body">
      <h4>{title}</h4>
      <pre>{text}</pre>
    </div>
  );
}

// One request row that expands to its redacted headers + bodies (the
// data is already in the fetched blob — a real inspector, no new fetch).
function HarRow({ entry }: { entry: HarEntry }) {
  const [open, setOpen] = useState(false);
  const ok = entry.response.status < 400;
  return (
    <>
      <tr
        className={`har-row${ok ? "" : " har-bad"}`}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setOpen((o) => !o);
          }
        }}
        tabIndex={0}
        role="button"
        aria-expanded={open}
        data-testid="har-row"
      >
        <td>
          {open ? (
            <ChevronDown className="har-caret" aria-hidden="true" />
          ) : (
            <ChevronRight className="har-caret" aria-hidden="true" />
          )}{" "}
          {entry.request.method}
        </td>
        <td className="har-url" title={entry.request.url}>
          {entry.request.url}
        </td>
        <td>
          <span className={`status-pill ${ok ? "ok" : "bad"}`}>{entry.response.status}</span>
        </td>
      </tr>
      {open && (
        <tr className="har-detail">
          <td colSpan={3}>
            <HarHeaders title="request headers" headers={entry.request.headers} />
            <HarBody title="request body" text={entry.request.postData?.text} />
            <HarHeaders title="response headers" headers={entry.response.headers} />
            <HarBody title="response body" text={entry.response.content?.text} />
          </td>
        </tr>
      )}
    </>
  );
}

// Render a fetched HAR blob as a request table — the network evidence
// read for humans, redaction markers intact, each row expandable.
export function HarTable({ url }: { url: string }) {
  const [entries, setEntries] = useState<HarEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    // Reset on url change so switching artifacts never shows stale rows.
    setEntries(null);
    setErr(null);
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((h) => {
        if (!live) return;
        const e = h?.log?.entries;
        setEntries(Array.isArray(e) ? e : []);
      })
      .catch((e) => live && setErr(String(e)));
    return () => {
      live = false;
    };
  }, [url]);
  if (err) return <p className="muted">could not load HAR: {err}</p>;
  if (entries === null) return <p className="muted">loading requests…</p>;
  if (entries.length === 0) return <p className="muted">no requests recorded</p>;
  // Wrapped so a long request URL / body scrolls *within the block*
  // instead of stretching the evidence panel and the whole page (#284
  // follow-up).
  return (
    <div className="har-wrap">
      <table className="har" data-testid="har-table">
        <thead>
          <tr>
            <th>method</th>
            <th>URL</th>
            <th>status</th>
          </tr>
        </thead>
        <tbody>
          {entries.map((e, i) => (
            <HarRow key={i} entry={e} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

// Inline viewer for a captured DOM snapshot: the HTML rendered in a
// fully sandboxed iframe (no scripts, no same-origin — the snapshot is
// untrusted page content) plus text search over the source, so you can
// ask "was this node ever present?" without downloading the blob.
export function DomViewer({ url }: { url: string }) {
  const [html, setHtml] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [showRender, setShowRender] = useState(false);
  useEffect(() => {
    let live = true;
    setHtml(null);
    setErr(null);
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.text();
      })
      .then((t) => live && setHtml(t))
      .catch((e) => live && setErr(String(e)));
    return () => {
      live = false;
    };
  }, [url]);
  // Lowercase the (potentially large) snapshot once, not on every
  // keystroke — the search box stays responsive.
  const haystack = useMemo(() => (html ?? "").toLowerCase(), [html]);
  if (err) return <p className="muted">could not load DOM: {err}</p>;
  if (html === null) return <p className="muted">loading DOM…</p>;
  const matches = q ? haystack.split(q.toLowerCase()).length - 1 : 0;
  return (
    <div className="dom-viewer" data-testid="dom-viewer">
      <div className="dom-search">
        <input
          type="search"
          placeholder="search the snapshot…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          aria-label="search the DOM snapshot"
        />
        {q && (
          <span className="muted" data-testid="dom-matches">
            {matches} match{matches === 1 ? "" : "es"}
          </span>
        )}
        <button
          type="button"
          className="dom-toggle"
          onClick={() => setShowRender((s) => !s)}
          aria-expanded={showRender}
          data-testid="dom-render-toggle"
        >
          {showRender ? "hide rendered snapshot" : "show rendered snapshot"}
        </button>
      </div>
      {showRender && (
        <>
          {/* sandbox="" disables scripts and same-origin — snapshot is untrusted. */}
          <iframe className="dom-frame" sandbox="" srcDoc={html} title="DOM snapshot" />
          <p className="muted dom-note">Rendered without external assets — structure and text, not pixels (see the screenshot for that).</p>
        </>
      )}
    </div>
  );
}

interface TargetRect {
  selector: string;
  expected: string;
  found: boolean;
  rect?: { x: number; y: number; width: number; height: number } | null;
}

// Image artifacts render as an inline thumbnail — a full-bleed
// screenshot dominates the panel otherwise. Click toggles to full
// size. When a `capture/target-rect` (#214) is available, the expanded
// view overlays "where the assertion looked" and notes absent targets.
export function ScreenshotArtifact({
  artifact,
  rectsUrl,
}: {
  artifact: ArtifactRef;
  rectsUrl?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [rects, setRects] = useState<TargetRect[]>([]);
  const [nat, setNat] = useState<{ w: number; h: number } | null>(null);
  useEffect(() => {
    if (!rectsUrl) {
      setRects([]);
      return;
    }
    let live = true;
    fetch(rectsUrl)
      .then((r) => r.json())
      .then((j) => live && setRects(Array.isArray(j) ? j : []))
      .catch(() => live && setRects([]));
    return () => {
      live = false;
    };
  }, [rectsUrl]);
  const found = rects.filter((r) => r.found && r.rect);
  const notFound = rects.filter((r) => !r.found);
  return (
    <>
      <button
        type="button"
        className={`shot-btn ${expanded ? "shot-expanded" : "shot-collapsed"}`}
        onClick={() => setExpanded((e) => !e)}
        aria-expanded={expanded}
        aria-label={expanded ? "collapse screenshot" : "expand screenshot to full size"}
        data-testid="shot-toggle"
      >
        <img
          src={artifact.url}
          alt={artifactLabel(artifact.kind)}
          onLoad={(e) => setNat({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
        />
        {/* Boxes map only on the natural-aspect expanded image. */}
        {expanded &&
          nat &&
          found.map((r, i) => (
            <span
              key={i}
              className="target-box"
              data-testid="target-box"
              title={r.selector}
              style={{
                left: `${(r.rect!.x / nat.w) * 100}%`,
                top: `${(r.rect!.y / nat.h) * 100}%`,
                width: `${(r.rect!.width / nat.w) * 100}%`,
                height: `${(r.rect!.height / nat.h) * 100}%`,
              }}
            />
          ))}
        <span className="shot-overlay">
          <span className="shot-cue">
            {expanded ? (
              <>
                <Minimize2 aria-hidden="true" /> Collapse
              </>
            ) : (
              <>
                <Maximize2 aria-hidden="true" /> Expand
              </>
            )}
          </span>
        </span>
      </button>
      {notFound.length > 0 && (
        <p className="muted target-note" data-testid="target-note">
          target not found on the page: {notFound.map((r) => r.selector).join(" · ")}
        </p>
      )}
    </>
  );
}

// The screencast of the check (#215). Rendered inline with native
// controls; the browser streams it from the artifact route. Opt-in
// capture, so it only appears when `--capture-video` recorded one.
export function VideoArtifact({ artifact }: { artifact: ArtifactRef }) {
  return (
    <video
      className="capture-video"
      data-testid="capture-video"
      src={artifact.url}
      controls
      preload="metadata"
    >
      <a href={artifact.url}>download video</a>
    </video>
  );
}

export function Artifacts({ artifacts }: { artifacts: CheckDetail["artifacts"] }) {
  // The target-rect is an overlay input for the screenshot (#214). Only
  // hide it from the list when a screenshot exists to overlay it onto —
  // otherwise (screenshot capture failed) keep it as its own row so the
  // evidence isn't lost.
  const hasScreenshot = artifacts.some((a) => isImageArtifact(a.kind, a.url));
  const rectsUrl = hasScreenshot
    ? artifacts.find((a) => a.kind === "capture/target-rect")?.url
    : undefined;
  const shown = hasScreenshot
    ? artifacts.filter((a) => a.kind !== "capture/target-rect")
    : artifacts;
  if (shown.length === 0) return null;
  return (
    <div className="panel">
      <h2>Artifacts</h2>
      {shown.map((artifact) => (
        <div className="artifact" key={artifact.id}>
          <p className="kv">
            <strong>{artifactLabel(artifact.kind)}</strong> ·{" "}
            <a href={artifact.url} target="_blank" rel="noreferrer">
              open<span className="muted"> ({artifact.id.slice(0, 12)}…)</span>
            </a>
          </p>
          {isImageArtifact(artifact.kind, artifact.url) && (
            <ScreenshotArtifact artifact={artifact} rectsUrl={rectsUrl} />
          )}
          {artifact.kind === "capture/network" && <HarTable url={artifact.url} />}
          {artifact.kind === "capture/dom" && <DomViewer url={artifact.url} />}
          {artifact.kind === "capture/video" && <VideoArtifact artifact={artifact} />}
        </div>
      ))}
    </div>
  );
}

export default function CheckPage() {
  const { runId = "", pair = "" } = useParams();
  // The check evidence renders inside the shared run tree, with this
  // check's node active in the rail. No back link — the rail (and the
  // breadcrumb) carry the way back.
  return (
    <RunScaffold runId={runId} activePair={pair}>
      {() => <CheckEvidence runId={runId} pair={pair} />}
    </RunScaffold>
  );
}

function CheckEvidence({ runId, pair }: { runId: string; pair: string }) {
  const [check, setCheck] = useState<CheckDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const [criterionId, checkId] = pair.split("::", 2);
    if (!criterionId || !checkId) {
      setError(`bad check reference: ${pair}`);
      return;
    }
    setCheck(null);
    setError(null);
    fetchCheck(runId, criterionId, checkId).then(setCheck, (e) => setError(String(e)));
  }, [runId, pair]);

  const vd = useVd();

  if (error) return <p className="error">{error}</p>;
  if (check === null) return <p className="muted">Loading…</p>;

  // The authored intent for this check + its criterion, from the recorded
  // VD snapshot (#302) — *what* this check verifies, not just its id.
  const checkDesc = vd?.check(check.criterion_id, check.check_id)?.description;
  const critDesc = vd?.criterion(check.criterion_id)?.description;

  return (
    <>
      <div className="panel">
        <h2>
          {check.criterion_id} :: {check.check_id}{" "}
          <VerdictBadge verdict={check.verdict} />
        </h2>
        {checkDesc && <p className="check-intent" data-testid="check-intent">{checkDesc}</p>}
        {critDesc && (
          <p className="kv crit-intent">
            <span className="muted">{check.criterion_id}: </span>
            {critDesc}
          </p>
        )}
        <SpanChain spans={check.spans} />
        <CheckSummary detail={check} />
        <Timeline events={check.timeline} artifacts={check.artifacts} />
      </div>
      <Artifacts artifacts={check.artifacts} />
    </>
  );
}
