// The recorded Verification Definition snapshot (#302): the raw VD YAML
// as it was when this run was judged, rendered in the full-width
// Definition tab. Makes a run self-describing — the criteria, checks,
// steps, and assertion rules are visible without the source file on hand.

import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";

import { definitionUrl, fetchDefinition } from "../api";
import { RunScaffold } from "./RunScaffold";

export default function DefinitionPage() {
  const { runId = "" } = useParams();
  return (
    <RunScaffold runId={runId} activeDefinition>
      {() => <DefinitionView runId={runId} />}
    </RunScaffold>
  );
}

function DefinitionView({ runId }: { runId: string }) {
  const [yaml, setYaml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setYaml(null);
    setError(null);
    fetchDefinition(runId).then(setYaml, (e) => setError(String(e)));
  }, [runId]);

  return (
    <div className="run-detail-surface">
      <h2 className="mb-2 text-base font-semibold">
        Verification Definition <span className="muted">— recorded snapshot</span>
      </h2>
      <p className="kv">
        The VD source as it was when this run was judged.{" "}
        <a href={definitionUrl(runId)} target="_blank" rel="noreferrer">
          raw
        </a>
      </p>
      {error ? (
        <p className="error">could not load definition: {error}</p>
      ) : yaml === null ? (
        <p className="muted">Loading…</p>
      ) : (
        <pre className="vd-yaml" data-testid="vd-yaml">
          {yaml}
        </pre>
      )}
    </div>
  );
}
