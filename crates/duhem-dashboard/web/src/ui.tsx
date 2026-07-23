// Small shared presentation helpers.

import { Badge } from "@/components/ui/badge";
import type { Verdict } from "./api";

// Verdict "family" collapses "inconclusive:<cause>" to "inconclusive".
export function verdictFamily(
  verdict: Verdict | null,
): "pass" | "fail" | "inconclusive" | null {
  if (verdict === null) return null;
  if (verdict === "pass") return "pass";
  if (verdict === "fail") return "fail";
  return "inconclusive";
}

// Legacy class hook — still used by the not-yet-reskinned evidence views
// (sparkline dots, diff rows). The badge itself is now a shadcn Badge.
export function verdictClass(verdict: Verdict | null): string {
  const fam = verdictFamily(verdict);
  return fam ? `verdict-${fam}` : "verdict-none";
}

export function VerdictBadge({
  verdict,
  live,
}: {
  verdict: Verdict | null;
  live?: boolean;
}) {
  if (verdict === null && live) {
    return (
      <Badge variant="live" className="gap-1.5">
        <span className="size-1.5 rounded-full bg-live motion-safe:animate-pulse" />
        live
      </Badge>
    );
  }
  if (verdict === null) {
    return <Badge variant="none">—</Badge>;
  }
  return <Badge variant={verdictFamily(verdict) ?? "none"}>{verdict}</Badge>;
}

export function formatStartedAt(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

export function formatDuration(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${Math.round(s % 60)}s`;
}

export function isImageArtifact(kind: string, url: string): boolean {
  return (
    kind.toLowerCase().includes("screenshot") || /\.(png|jpe?g)$/i.test(url)
  );
}
