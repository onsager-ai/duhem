import { Link } from "react-router-dom";
import type { ShardCoverage } from "../api";
import { formatDuration } from "../ui";

export function Shards({ coverage }: { coverage: ShardCoverage }) {
  const missingStatus = coverage.status === "pending" ? "pending" : "inconclusive";
  return (
    <section aria-labelledby="shards-heading" className="space-y-3">
      <h3 id="shards-heading" className="text-sm font-semibold">Shards</h3>
      <p className="text-sm text-muted-foreground">
        {coverage.arrived.length}/{coverage.total} arrived · {coverage.units.length} units covered
      </p>
      <ol aria-label="Shards" className="divide-y rounded-md border">
        {coverage.arrived.map((shard) => (
          <li key={shard.index} className="flex flex-wrap gap-3 px-3 py-2 text-sm">
            <Link className="font-medium hover:underline" to={`/run/${encodeURIComponent(shard.run_id)}`}>
              Shard {shard.index}/{shard.total}
            </Link>
            <span>{shard.verdict ?? "pending"}</span>
            <span>{shard.duration_ms === null ? "duration unavailable" : formatDuration(shard.duration_ms)}</span>
            <span>Source: {shard.source}</span>
            <span>{shard.unit_count} units</span>
          </li>
        ))}
        {coverage.missing.map((shard) => (
          <li key={shard.index} className="space-y-1 px-3 py-2 text-sm">
            <p>Shard {shard.index}/{coverage.total} · {missingStatus} · missing</p>
            <ul aria-label={`Missing units for shard ${shard.index}`} className="break-all text-muted-foreground">
              {shard.units.map((unit) => <li key={unit}>{unit}</li>)}
            </ul>
          </li>
        ))}
      </ol>
    </section>
  );
}
