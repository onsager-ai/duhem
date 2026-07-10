// Per-check evidence (#86, #206): a plain-language summary, then the
// check's slice of the trace rendered as legible rows (icon · label ·
// detail · Δ) with the raw JSON one click away, and a rich artifacts
// panel (screenshots inline, network HAR as a request table).

import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { fetchCheck, type ArtifactRef, type CheckDetail, type SpanModel, type TraceEvent } from "../api";
import { VerdictBadge, isImageArtifact } from "../ui";
import { formatEvent, summarizeCheck } from "../format";

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
      <span className="ev-body">
        <span className="ev-label">{fe.label}</span>
        {fe.detail && <span className="ev-detail">{fe.detail}</span>}
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

export function Timeline({
  events,
  artifacts = [],
}: {
  events: TraceEvent[];
  artifacts?: ArtifactRef[];
}) {
  return (
    <ol className="timeline">
      {events.map((evt, i) => (
        <TimelineRow key={evt.seq} evt={evt} prev={events[i - 1]} artifacts={artifacts} />
      ))}
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
    default:
      return kind;
  }
}

interface HarEntry {
  request: { method: string; url: string; postData?: { text?: string } };
  response: { status: number; content?: { text?: string } };
}

// Render a fetched HAR blob as a compact request table — the network
// evidence read for humans, redaction markers intact.
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
        {entries.map((e, i) => {
          const ok = e.response.status < 400;
          return (
            <tr key={i} className={ok ? "" : "har-bad"}>
              <td>{e.request.method}</td>
              <td className="har-url" title={e.request.url}>
                {e.request.url}
              </td>
              <td>{e.response.status}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

export function Artifacts({ artifacts }: { artifacts: CheckDetail["artifacts"] }) {
  if (artifacts.length === 0) return null;
  return (
    <div className="panel">
      <h2>Artifacts</h2>
      {artifacts.map((artifact) => (
        <div className="artifact" key={artifact.id}>
          <p className="kv">
            <strong>{artifactLabel(artifact.kind)}</strong> ·{" "}
            <a href={artifact.url} target="_blank" rel="noreferrer">
              open<span className="muted"> ({artifact.id.slice(0, 12)}…)</span>
            </a>
          </p>
          {isImageArtifact(artifact.kind, artifact.url) && (
            <img src={artifact.url} alt={`${artifactLabel(artifact.kind)}`} />
          )}
          {artifact.kind === "capture/network" && <HarTable url={artifact.url} />}
        </div>
      ))}
    </div>
  );
}

export default function CheckPage() {
  const { runId = "", pair = "" } = useParams();
  const [check, setCheck] = useState<CheckDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const [criterionId, checkId] = pair.split("::", 2);
    if (!criterionId || !checkId) {
      setError(`bad check reference: ${pair}`);
      return;
    }
    fetchCheck(runId, criterionId, checkId).then(setCheck, (e) => setError(String(e)));
  }, [runId, pair]);

  if (error) return <p className="error">{error}</p>;
  if (check === null) return <p className="muted">Loading…</p>;

  return (
    <>
      <p className="kv">
        <Link to={`/run/${encodeURIComponent(runId)}`}>← run {runId}</Link>
      </p>
      <div className="panel">
        <h2>
          {check.criterion_id} :: {check.check_id}{" "}
          <VerdictBadge verdict={check.verdict} />
        </h2>
        <SpanChain spans={check.spans} />
        <CheckSummary detail={check} />
        <Timeline events={check.timeline} artifacts={check.artifacts} />
      </div>
      <Artifacts artifacts={check.artifacts} />
    </>
  );
}
