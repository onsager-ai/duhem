// Per-check evidence (#86, #206): a plain-language summary, then the
// check's slice of the trace rendered as legible rows (icon · label ·
// detail · Δ) with the raw JSON one click away, and a rich artifacts
// panel (screenshots inline, network HAR as a request table).

import { Fragment, useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  fetchCheck,
  fetchRun,
  type ArtifactRef,
  type CheckDetail,
  type RunDetail,
  type SpanModel,
  type TraceEvent,
} from "../api";
import { VerdictBadge, formatDuration, isImageArtifact } from "../ui";
import { formatEvent, groupTimeline, stepStatus, summarizeCheck, type TimelineNode } from "../format";
import { RunTabs } from "./RunPage";

// The check's wall-clock span — first recorded event to last.
function checkDurationMs(timeline: TraceEvent[]): number | null {
  if (timeline.length < 2) return null;
  const a = Date.parse(timeline[0].ts);
  const b = Date.parse(timeline[timeline.length - 1].ts);
  if (Number.isNaN(a) || Number.isNaN(b)) return null;
  return b - a;
}

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
  return (
    <li className={`ev tone-${fe.tone}`}>
      <span className="ev-icon" aria-hidden="true">
        {fe.icon}
      </span>
      <span className="ev-label">{fe.label}</span>
      <span className="ev-detail">
        {fe.detail && (
          <span className="ev-detail-text" title={fe.detail}>
            {fe.detail}
          </span>
        )}
        {art && (
          <a className="ev-artifact" href={art.url} target="_blank" rel="noreferrer">
            open
          </a>
        )}
        <details className="ev-raw">
          <summary>raw</summary>
          <pre>{fe.raw}</pre>
        </details>
      </span>
      <span className="ev-time" title={evt.ts}>
        {fe.delta ?? ""}
      </span>
    </li>
  );
}

// Pretty-print a value for the request/response inspector: JSON strings
// and objects are re-indented; anything else is shown as-is.
function pretty(v: unknown): string {
  if (typeof v === "string") {
    try {
      return JSON.stringify(JSON.parse(v), null, 2);
    } catch {
      return v;
    }
  }
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

// Request/response inspector for an `api/*` (or `db/*`) step — the method
// · url · response status on one line, with the request and response
// bodies pretty-printed and collapsible (the response opens on a 4xx/5xx
// so the failure body — often the root cause — is right there). Reads the
// request from the step's `with:` and the response from its observations;
// returns null for a step that isn't an HTTP exchange.
function ApiExchange({ node }: { node: Extract<TimelineNode, { kind: "step" }> }) {
  const started = node.events[0];
  const w =
    started.with && typeof started.with === "object"
      ? (started.with as Record<string, unknown>)
      : {};
  const obs = (name: string): unknown =>
    node.events.find(
      (e) =>
        (e.kind === "step_observation" || e.kind === "setup_step_observation") &&
        e.output_name === name,
    )?.value;
  const method = typeof w.method === "string" ? w.method : undefined;
  const url = typeof w.url === "string" ? w.url : undefined;
  const reqBody = w.body;
  const status = obs("status");
  const respBody = obs("body") ?? obs("body_text");
  if (url === undefined && status === undefined) return null;
  const bad = typeof status === "number" && status >= 400;
  return (
    <div className="api-exchange" data-testid="api-exchange">
      <div className="ax-line">
        {method && <span className="ax-method">{method}</span>}{" "}
        {url && <span className="ax-url">{url}</span>}
        {status !== undefined && (
          <span className={`ax-status ${bad ? "bad" : "ok"}`}>{String(status)}</span>
        )}
      </div>
      {reqBody !== undefined && reqBody !== null && reqBody !== "" && (
        <details className="ax-body">
          <summary>request body</summary>
          <pre>{pretty(reqBody)}</pre>
        </details>
      )}
      {respBody !== undefined && respBody !== null && respBody !== "" && (
        <details className="ax-body" open={bad}>
          <summary>response body</summary>
          <pre>{pretty(respBody)}</pre>
        </details>
      )}
    </div>
  );
}

// Evidence captured on this step (#280 polish): screenshot / DOM /
// network / video blobs recorded as `capture/*` observations, nested
// under the step that produced them (Allure-style) instead of a side
// panel — reusing the same viewers.
function StepCaptures({
  node,
  artifacts,
}: {
  node: Extract<TimelineNode, { kind: "step" }>;
  artifacts: ArtifactRef[];
}) {
  const caps = node.events
    .filter(
      (e) =>
        (e.kind === "step_observation" || e.kind === "setup_step_observation") &&
        typeof e.blob_sha256 === "string" &&
        typeof e.output_name === "string" &&
        (e.output_name as string).startsWith("capture/"),
    )
    .map((e) => ({
      kind: e.output_name as string,
      art: artifacts.find((a) => a.id === e.blob_sha256),
    }))
    .filter((c): c is { kind: string; art: ArtifactRef } => c.art !== undefined);
  if (caps.length === 0) return null;
  // The target-rect capture is an overlay input for the screenshot, not a
  // row of its own (mirrors the old Artifacts panel).
  const shot = caps.find((c) => isImageArtifact(c.art.kind, c.art.url));
  const rectsUrl = shot
    ? caps.find((c) => c.kind === "capture/target-rect")?.art.url
    : undefined;
  const shown = shot ? caps.filter((c) => c.kind !== "capture/target-rect") : caps;
  return (
    <div className="step-captures" data-testid="step-captures">
      {shown.map((c) => (
        <div className="artifact" key={c.art.id}>
          <p className="kv">
            <strong>{artifactLabel(c.art.kind)}</strong> ·{" "}
            <a href={c.art.url} target="_blank" rel="noreferrer">
              open
            </a>
          </p>
          {isImageArtifact(c.art.kind, c.art.url) && (
            <ScreenshotArtifact artifact={c.art} rectsUrl={rectsUrl} />
          )}
          {c.kind === "capture/network" && <HarTable url={c.art.url} />}
          {c.kind === "capture/dom" && <DomViewer url={c.art.url} />}
          {c.kind === "capture/video" && <VideoArtifact artifact={c.art} />}
        </div>
      ))}
    </div>
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
  const observations = node.events.filter(
    (e) => e.kind === "step_observation" || e.kind === "setup_step_observation",
  );
  const fe = formatEvent(started, prevOf(started));
  const status = stepStatus(node);
  // Surface an api/call's HTTP response status right on the step row, so a
  // failing call (→ 500) is obvious at a glance instead of buried in "N obs".
  const statusObs = observations.find(
    (e) => e.output_name === "status" && typeof e.value === "number",
  );
  const httpStatus = statusObs ? (statusObs.value as number) : undefined;
  return (
    <li className={`ev step-group tone-${status.tone}`} data-testid="step-group">
      <details open={status.failed}>
        <summary>
          <span className="ev-icon" aria-hidden="true">
            {fe.icon}
          </span>
          <span className="ev-label">{fe.label}</span>
          <span className="ev-detail">
            {fe.detail && (
              <span className="ev-detail-text" title={fe.detail}>
                {fe.detail}
              </span>
            )}
            {httpStatus !== undefined && (
              <span
                className={`api-status ${httpStatus >= 400 ? "bad" : "ok"}`}
                data-testid="api-status"
              >
                → {httpStatus}
              </span>
            )}
            <span className={`step-outcome tone-${status.tone}`} data-testid="step-outcome">
              {status.icon} {status.label}
            </span>
            {observations.length > 0 && (
              <span className="obs-count">{observations.length} obs</span>
            )}
            {status.reason && (
              <span className="step-reason" data-testid="step-reason">
                {status.reason}
              </span>
            )}
          </span>
          <span className="ev-time" title={started.ts}>
            {fe.delta ?? ""}
          </span>
        </summary>
        {/* For an api/db step, a legible request/response inspector —
            method · url · status + pretty-printed bodies — nested under
            the step (the api analogue of a screenshot attachment). */}
        <ApiExchange node={node} />
        {/* ui evidence (screenshot / DOM / network / video) nested under
            the step that captured it, not a side panel. */}
        <StepCaptures node={node} artifacts={artifacts} />
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
          {i > 0 && <span className="muted"> → </span>}
          <span
            className={`span-node ${s.ok ? "span-ok" : "span-fail"}`}
            title={`step evidence #${s.seq}${s.detail ? ` — ${s.detail}` : ""}`}
          >
            {s.layer}
            {!s.ok && s.detail ? ` ✕ ${s.detail}` : !s.ok ? " ✕" : ""}
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
          <span className="har-caret" aria-hidden="true">
            {open ? "▾" : "▸"}
          </span>{" "}
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
  return (
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
          <span className="shot-cue">{expanded ? "Collapse" : "⤢  Expand"}</span>
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
  const [check, setCheck] = useState<CheckDetail | null>(null);
  // The run is fetched only for the shell header (verification + verdict);
  // the check page keeps the run-scoped tab bar so drilling into a check
  // doesn't drop you out of the report shell.
  const [run, setRun] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const [criterionId, checkId] = pair.split("::", 2);
    if (!criterionId || !checkId) {
      setError(`bad check reference: ${pair}`);
      return;
    }
    fetchCheck(runId, criterionId, checkId).then(setCheck, (e) => setError(String(e)));
    fetchRun(runId).then(setRun, () => {});
  }, [runId, pair]);

  if (error) return <p className="error">{error}</p>;
  if (check === null) return <p className="muted">Loading…</p>;

  return (
    <>
      <p className="kv">
        <Link to="/">← runs</Link>
      </p>
      <h2 className="run-title">
        {run ? `${run.verification} · ` : ""}
        <code>{runId}</code>{" "}
        {run && <VerdictBadge verdict={run.verdict} live={run.live} />}
      </h2>
      {/* Keep the run-scoped tabs on the check page (Suites is where a
          check is reached from), so the report shell persists. */}
      <RunTabs runId={runId} active="suites" />
      <div className="panel">
        <p className="kv check-crumb">
          <Link to={`/run/${encodeURIComponent(runId)}/suites`}>Suites</Link> ›{" "}
          {check.criterion_id}
        </p>
        <h2>
          {check.criterion_id} :: {check.check_id}{" "}
          <VerdictBadge verdict={check.verdict} />
        </h2>
        <p className="kv check-meta" data-testid="check-meta">
          {(() => {
            const dur = checkDurationMs(check.timeline);
            const steps = check.timeline.filter((e) => e.kind === "step_started").length;
            const layers = check.spans.length;
            return [
              dur !== null ? `⏱ ${formatDuration(dur)}` : null,
              `${steps} step${steps === 1 ? "" : "s"}`,
              layers > 0 ? `${layers} layer${layers === 1 ? "" : "s"}` : null,
            ]
              .filter(Boolean)
              .join(" · ");
          })()}
        </p>
        <SpanChain spans={check.spans} />
        <CheckSummary detail={check} />
        {/* Captures now nest under the step that produced them (see
            StepCaptures); no separate Artifacts panel. */}
        <Timeline events={check.timeline} artifacts={check.artifacts} />
      </div>
    </>
  );
}
