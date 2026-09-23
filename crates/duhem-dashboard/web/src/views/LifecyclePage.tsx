import { useParams } from "react-router-dom";
import type { LifecycleBlock, RunDetail } from "../api";
import { lifecycleFailure, lifecycleKey, lifecycleScopePath } from "../lifecycle";
import { formatDuration } from "../ui";
import { LifecycleStatusBadge } from "../components/LifecycleStatusBadge";
import { Timeline } from "./CheckPage";
import { RunScaffold } from "./RunScaffold";

export function LifecycleEvidence({ block }: { block: LifecycleBlock }) {
  const failure = lifecycleFailure(block);
  const failing = block.failing_step === undefined ? undefined : block.steps[block.failing_step];
  return (
    <div className="run-detail-surface min-w-0 space-y-4" data-testid="lifecycle-detail">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-base font-semibold break-all">{lifecycleScopePath(block)} / {block.phase}</h2>
        <LifecycleStatusBadge status={block.status} />
      </div>
      <dl className="grid gap-3 text-sm sm:grid-cols-2">
        <div><dt className="text-muted-foreground">Scope</dt><dd className="font-mono break-all">{lifecycleScopePath(block)}</dd></div>
        <div><dt className="text-muted-foreground">Duration</dt><dd>{formatDuration(block.duration_ms)}</dd></div>
      </dl>
      {failing && (
        <div className="rounded-md border border-fail/30 bg-fail/5 p-3 text-sm" data-testid="lifecycle-failure">
          <strong>Failed step {failing.index + 1}: {failing.uses}</strong>
          {failure && <p className="mt-1 whitespace-pre-wrap break-words text-fail">{failure}</p>}
        </div>
      )}
      <Timeline events={block.timeline} />
    </div>
  );
}

function LifecycleFromRun({ run, keyValue }: { run: RunDetail; keyValue: string }) {
  const block = (run.lifecycle ?? []).find((item) => lifecycleKey(item) === keyValue && item.scope.length <= 1);
  return block ? <LifecycleEvidence block={block} /> : <p className="error">Lifecycle block not found.</p>;
}

export default function LifecyclePage() {
  const { runId = "", blockKey = "" } = useParams();
  return (
    <RunScaffold runId={runId} activeResults activeLifecycle={blockKey}>
      {(run) => <LifecycleFromRun run={run} keyValue={blockKey} />}
    </RunScaffold>
  );
}
