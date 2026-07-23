// One shared, live fetch of `api/runs.json` for the whole shell. Overview,
// the Runs table, the Verifications index, the ⌘K palette, and the live
// indicator all read from here, so navigating between them is instant and
// the list stays fresh from a single source. Freshness is the
// visibility-aware poll from `usePolledRuns` (#298/#303), folded into the
// shell context so the top-bar live indicator updates too — not just /runs.

import { createContext, useContext, type ReactNode } from "react";

import { type RunsListEntry } from "./api";
import { usePolledRuns } from "./hooks/use-polled-runs";

interface RunsData {
  runs: RunsListEntry[] | null;
  error: string | null;
}

const RunsContext = createContext<RunsData | null>(null);

export function RunsProvider({ children }: { children: ReactNode }) {
  const { runs, error } = usePolledRuns();
  return (
    <RunsContext.Provider value={{ runs, error }}>
      {children}
    </RunsContext.Provider>
  );
}

export function useRunsData(): RunsData {
  const ctx = useContext(RunsContext);
  if (!ctx) throw new Error("useRunsData must be used within a RunsProvider");
  return ctx;
}
