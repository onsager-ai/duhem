// Live runs list (#298): poll `api/runs.json` while the tab is
// visible, so in-flight runs appear and verdicts resolve without a
// manual reload. The per-run page already streams (SSE); this is the
// on-ramp — the list is where an operator notices a run exists.
//
// Polling, not SSE, at list granularity: the list changes at run
// cadence (seconds), the server recomputes it per request anyway, and
// a hidden tab pays nothing. A visibilitychange listener refreshes
// immediately on return so the first paint is never stale.

import { useEffect, useRef, useState } from "react";
import { fetchRuns, type RunsListEntry } from "../api";

export const POLL_INTERVAL_MS = 3000;

export interface PolledRuns {
  runs: RunsListEntry[] | null;
  error: string | null;
}

export function usePolledRuns(intervalMs: number = POLL_INTERVAL_MS): PolledRuns {
  const [runs, setRuns] = useState<RunsListEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Whether any fetch has succeeded: a failed *background* refresh
  // keeps showing the last good list instead of flashing an error.
  const hasData = useRef(false);

  useEffect(() => {
    let disposed = false;

    const refresh = () => {
      fetchRuns().then(
        (entries) => {
          if (disposed) return;
          hasData.current = true;
          setRuns(entries);
          setError(null);
        },
        (e) => {
          if (disposed || hasData.current) return;
          setError(String(e));
        },
      );
    };

    const tick = () => {
      if (document.visibilityState === "visible") refresh();
    };
    const onVisible = () => {
      if (document.visibilityState === "visible") refresh();
    };

    refresh();
    const timer = setInterval(tick, intervalMs);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      disposed = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [intervalMs]);

  return { runs, error };
}
