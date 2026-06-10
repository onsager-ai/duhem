// Per-check timeline (#86): the check's slice of the trace in event
// order, with observations inline and blob artifacts rendered
// (screenshots as images, anything else as a link).

import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { fetchCheck, type CheckDetail, type TraceEvent } from "../api";
import { VerdictBadge, isImageArtifact } from "../ui";

function eventDetail(evt: TraceEvent): string {
  const { seq: _seq, ts: _ts, kind: _kind, ...rest } = evt;
  const entries = Object.entries(rest);
  return entries.length === 0 ? "" : JSON.stringify(rest, null, 1);
}

export function Timeline({ events }: { events: TraceEvent[] }) {
  return (
    <ol className="timeline">
      {events.map((evt) => (
        <li key={evt.seq}>
          <span className="seq">#{evt.seq}</span>
          <span className="kind">{evt.kind}</span>
          <span>
            <span className="muted">{evt.ts}</span>
            {eventDetail(evt) && <pre>{eventDetail(evt)}</pre>}
          </span>
        </li>
      ))}
    </ol>
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
            {artifact.kind} ·{" "}
            <a href={artifact.url} target="_blank" rel="noreferrer">
              <code>{artifact.id.slice(0, 12)}…</code>
            </a>
          </p>
          {isImageArtifact(artifact.kind, artifact.url) && (
            <img src={artifact.url} alt={`${artifact.kind} ${artifact.id}`} />
          )}
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
        <Timeline events={check.timeline} />
      </div>
      <Artifacts artifacts={check.artifacts} />
    </>
  );
}
