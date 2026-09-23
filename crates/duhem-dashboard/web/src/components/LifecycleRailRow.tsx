import { CircleDot } from "lucide-react";
import { Link } from "react-router-dom";
import type { LifecycleBlock } from "../api";
import { lifecycleHref, lifecycleKey, lifecycleScopePath } from "../lifecycle";
import { cn } from "@/lib/utils";
import { LifecycleStatusBadge } from "./LifecycleStatusBadge";

export function LifecycleRailRow({
  runId, block, checkPath, active,
}: {
  runId: string;
  block: LifecycleBlock;
  checkPath?: string;
  active?: boolean;
}) {
  const scope = lifecycleScopePath(block);
  const label = block.scope.at(-1)?.kind === "fixture"
    ? `fixture ${block.scope.at(-1)?.id} ${block.phase}`
    : block.phase;
  const to = checkPath
    ? `${checkPath}?lifecycle=${encodeURIComponent(lifecycleKey(block))}`
    : lifecycleHref(runId, block);
  return (
    <Link
      to={to}
      aria-label={`${scope} ${block.phase} ${block.status}`}
      aria-current={active ? "page" : undefined}
      data-testid="lifecycle-rail-row"
      data-status={block.status}
      className={cn(
        "flex min-w-0 items-center gap-1.5 rounded px-2 py-1 text-xs text-muted-foreground hover:bg-accent/60 hover:text-foreground",
        active && "bg-accent text-accent-foreground",
      )}
    >
      <CircleDot className="size-3 shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate" title={scope}>{label}</span>
      <LifecycleStatusBadge status={block.status} />
    </Link>
  );
}
