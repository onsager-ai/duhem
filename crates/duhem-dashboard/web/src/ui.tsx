// Small shared presentation helpers.

import type { Verdict } from "./api";

export function verdictClass(verdict: Verdict | null): string {
  if (verdict === null) return "verdict-none";
  if (verdict === "pass") return "verdict-pass";
  if (verdict === "fail") return "verdict-fail";
  return "verdict-inconclusive";
}

export function VerdictBadge({
  verdict,
  live,
}: {
  verdict: Verdict | null;
  live?: boolean;
}) {
  if (verdict === null && live) {
    return <span className="badge badge-live">● live</span>;
  }
  if (verdict === null) {
    return <span className="badge verdict-none">—</span>;
  }
  return <span className={`badge ${verdictClass(verdict)}`}>{verdict}</span>;
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
    kind.toLowerCase().includes("screenshot") ||
    /\.(png|jpe?g)$/i.test(url)
  );
}
