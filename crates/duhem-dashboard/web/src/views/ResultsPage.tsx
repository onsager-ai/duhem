import { useParams } from "react-router-dom";

import { RunScaffold } from "./RunScaffold";

export default function ResultsPage() {
  const { runId = "" } = useParams();
  return (
    <RunScaffold runId={runId} activeResults>
      {() => (
        <div className="py-6 text-sm text-muted-foreground">
          Select a criterion or check to inspect its result.
        </div>
      )}
    </RunScaffold>
  );
}
