import { Link } from "react-router-dom";
import type { ArtifactRef, FailingNative, RunDetail } from "../api";
import { formatDuration } from "../ui";

export function NativeEvidence({ evidence }: { evidence: ArtifactRef[] }) {
  return (
    <span className="inline-flex flex-wrap gap-3">
      {evidence.map((artifact) => (
        <a key={artifact.kind} href={artifact.url.replace(/^\//, "")} className="underline">
          {artifact.kind}
        </a>
      ))}
    </span>
  );
}

export function NativeFailures({ failures }: { failures: FailingNative[] }) {
  if (!failures.length) return null;
  return (
    <ol aria-label="Native failures" className="space-y-2">
      {failures.map((failure) => (
        <li key={`${failure.run_id}:${failure.native_id}`} className="space-y-1 text-sm">
          <Link to={`/run/${encodeURIComponent(failure.run_id)}`} className="underline">
            {failure.native_id}
          </Link>
          {" · "}{failure.outcome}{" · "}{failure.required ? "Required" : "Diagnostic"}
          {failure.detail && <p className="whitespace-pre-wrap break-words">{failure.detail}</p>}
          <NativeEvidence evidence={failure.evidence} />
        </li>
      ))}
    </ol>
  );
}

export function NativeRuns({ run }: { run: RunDetail }) {
  if (!run.native?.length) return null;
  return (
    <>
      <section aria-label="Native runs" className="space-y-3">
        <h3 className="text-sm font-semibold">Native runs</h3>
        <p className="text-sm text-muted-foreground">The project’s own tests and tools. Required runs gate this run; they do not satisfy criteria.</p>
        <ol className="space-y-3">
          {run.native.map((native) => (
            <li key={`${native.run_id}:${native.id}`} className="space-y-2 border-l-2 pl-4 text-sm">
              <p className="flex flex-wrap gap-2">
                <span className="font-medium">{native.id}</span>
                <span>{native.layer ?? "other"}</span>
                <span>{native.outcome}</span>
                <span>{native.required ? "Required" : "Diagnostic"}</span>
                <span>{formatDuration(native.duration_ms)}</span>
              </p>
              <p>{native.tests} tests{native.counts && ` · ${native.counts.passed} passed · ${native.counts.failed} failed · ${native.counts.errored} errored · ${native.counts.skipped} skipped`}</p>
              {native.detail && <p className="whitespace-pre-wrap break-words">{native.detail}</p>}
              <NativeEvidence evidence={native.evidence} />
              {!!native.failing_tests?.length && (
                <ul aria-label={`Failing tests for ${native.id}`} className="space-y-2">
                  {native.failing_tests.map((test) => (
                    <li key={test.test_id} className="break-words">
                      <p>{test.name}{test.file && ` · ${test.file}`}</p>
                      {test.message && <p className="text-muted-foreground">{test.message}</p>}
                      <NativeEvidence evidence={native.evidence.filter((artifact) => artifact.kind === "results" || artifact.kind === "report")} />
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ol>
      </section>
      {!!run.layers?.length && (
        <section aria-label="Layer totals" className="space-y-3">
          <h3 className="text-sm font-semibold">Layer totals</h3>
          <div className="overflow-x-auto">
            <table aria-label="Layer totals" className="w-full text-left text-sm">
              <thead><tr>{["Layer", "Tests", "Passed", "Failed", "Errored", "Skipped", "Inconclusive", "Duration"].map((label) => <th key={label} scope="col" className="px-2 py-1">{label}</th>)}</tr></thead>
              <tbody>{run.layers.map((layer) => (
                <tr key={layer.layer}>
                  <th scope="row" className="px-2 py-1">{layer.layer}</th>
                  {[layer.tests, layer.passed, layer.failed, layer.errored ?? 0, layer.skipped ?? 0, layer.inconclusive, formatDuration(layer.duration_ms)].map((value, index) => <td key={index} className="px-2 py-1">{value}</td>)}
                </tr>
              ))}</tbody>
            </table>
          </div>
        </section>
      )}
    </>
  );
}
