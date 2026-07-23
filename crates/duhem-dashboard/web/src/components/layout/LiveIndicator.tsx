import { Link } from "react-router-dom";

import { useRunsData } from "@/runs-context";
import { flatLeaves } from "@/stats";

// Live-run pulse in the top bar. Links to the runs list filtered to live.
export function LiveIndicator() {
  const { runs } = useRunsData();
  if (!runs) return null;
  const live = flatLeaves(runs).filter((r) => r.live).length;

  if (live === 0) {
    return (
      <span className="hidden items-center gap-1.5 text-xs text-muted-foreground sm:flex">
        <span className="size-1.5 rounded-full bg-muted-foreground/40" />
        idle
      </span>
    );
  }
  return (
    <Link
      to="/runs?verdict=live"
      className="flex items-center gap-1.5 rounded-full border border-live/30 bg-live/10 px-2.5 py-1 text-xs font-medium text-live"
    >
      <span className="size-1.5 rounded-full bg-live motion-safe:animate-pulse" />
      {live} live
    </Link>
  );
}
