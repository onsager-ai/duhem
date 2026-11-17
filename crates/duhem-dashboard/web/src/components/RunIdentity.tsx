// A run's display identity (#694): the per-verification `#N` beside a
// copy button for the full ULID. Runs from one batch share their ULID
// timestamp prefix, so a truncated ULID does not tell them apart; the
// number does. Links and URLs keep the ULID, which stays the identifier.

import { Copy } from "lucide-react";
import { Link } from "react-router-dom";
import { toast } from "sonner";

import { cn } from "@/lib/utils";

export function runNumberLabel(run: { run_number: number }): string {
  return `#${run.run_number}`;
}

/** `#N` as a link to the run, the full ULID in its tooltip. */
export function RunNumberLink({
  run,
  className,
}: {
  run: { run_id: string; run_number: number };
  className?: string;
}) {
  return (
    <Link
      to={`/run/${encodeURIComponent(run.run_id)}`}
      title={run.run_id}
      className={cn("font-mono text-xs tabular-nums hover:underline", className)}
    >
      {runNumberLabel(run)}
    </Link>
  );
}

export function CopyRunId({ runId, className }: { runId: string; className?: string }) {
  const copy = () => {
    // Clipboard is unavailable outside secure contexts; say so rather
    // than claim a copy that did not happen.
    const clipboard = typeof navigator === "undefined" ? undefined : navigator.clipboard;
    if (!clipboard) {
      toast.error("Clipboard unavailable");
      return;
    }
    clipboard.writeText(runId).then(
      () => toast.success("Run ID copied"),
      () => toast.error("Could not copy run ID"),
    );
  };
  return (
    <button
      type="button"
      onClick={copy}
      aria-label="Copy run ID"
      data-run-id={runId}
      className={cn(
        "inline-grid size-5 shrink-0 place-items-center rounded text-muted-foreground hover:bg-muted hover:text-foreground",
        className,
      )}
    >
      <Copy className="size-3" aria-hidden="true" />
    </button>
  );
}
