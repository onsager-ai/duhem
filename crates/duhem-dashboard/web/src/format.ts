// Human-readable rendering of trace events (#206). Pure functions over
// the wire-format `TraceEvent` — the check page's timeline and summary
// are derived here, never re-judged and never LLM-authored. The raw
// JSON stays one click away in the UI; this is the legible default.

import type { CheckDetail, TraceEvent } from "./api";

export type Tone = "ok" | "fail" | "inconclusive" | "muted" | "anchor";

export interface FormattedEvent {
  icon: string;
  label: string;
  detail: string;
  tone: Tone;
  /** `+4.0s` relative to the previous event, or null for the first. */
  delta: string | null;
  /** Pretty payload (minus seq/ts/kind) for the row's raw toggle. */
  raw: string;
  /** Set for a blob observation so the row can link the artifact. */
  blobSha?: string;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

function toneOfState(state: string): Tone {
  if (state === "pass") return "ok";
  if (state === "fail") return "fail";
  return "inconclusive";
}

/** Compact one-line value for an inline observation or arg scalar. */
function compactValue(v: unknown): string {
  if (v === null) return "null";
  if (typeof v === "string") return v.length > 80 ? `${v.slice(0, 80)}…` : v;
  if (typeof v === "object") {
    const s = JSON.stringify(v);
    return s.length > 80 ? `${s.slice(0, 80)}…` : s;
  }
  return String(v);
}

/** A role/name/text/css locator as `role=button, text "Go"`. */
function describeLocator(loc: Record<string, unknown>): string {
  const parts: string[] = [];
  if (str(loc.role)) parts.push(`role=${loc.role}`);
  if (str(loc.name)) parts.push(`name "${loc.name}"`);
  if (str(loc.text)) parts.push(`text "${loc.text}"`);
  if (str(loc.css)) parts.push(`css ${loc.css}`);
  if (loc.scope && typeof loc.scope === "object") {
    parts.push(`in {${describeLocator(loc.scope as Record<string, unknown>)}}`);
  }
  return parts.join(", ");
}

/** The meaningful bits of a step's `with:` payload, human-ordered. */
export function describeWith(withObj: unknown): string {
  if (!withObj || typeof withObj !== "object") return "";
  const w = withObj as Record<string, unknown>;
  const parts: string[] = [];
  if (str(w.url)) parts.push(w.url as string);
  if (str(w.method) && str(w.url) === undefined) parts.push(w.method as string);
  if (w.locator && typeof w.locator === "object") {
    parts.push(describeLocator(w.locator as Record<string, unknown>));
  }
  if (str(w.text) && !w.locator) parts.push(`"${w.text}"`);
  if (str(w.expected)) parts.push(String(w.expected));
  if (str(w.within)) parts.push(`within ${w.within}`);
  if (parts.length === 0) {
    // Generic fallback: first couple of scalar fields.
    for (const [k, v] of Object.entries(w)) {
      if (typeof v !== "object") parts.push(`${k}=${compactValue(v)}`);
      if (parts.length >= 2) break;
    }
  }
  return parts.join(" · ");
}

/** Strip the `ui/` / `api/` / `db/` family prefix from a `uses`. */
function actionVerb(uses: string): string {
  const slash = uses.indexOf("/");
  return slash >= 0 ? uses.slice(slash + 1) : uses;
}

/** Friendly label for a `capture/*` (or other) blob observation. */
function blobLabel(outputName: string): { icon: string; label: string } {
  switch (outputName) {
    case "capture/screenshot":
      return { icon: "📷", label: "screenshot captured" };
    case "capture/dom":
      return { icon: "📄", label: "DOM captured" };
    case "capture/network":
      return { icon: "🌐", label: "network captured" };
    default:
      return { icon: "📎", label: `${outputName} captured` };
  }
}

function relTime(evt: TraceEvent, prev: TraceEvent | undefined): string | null {
  if (!prev) return null;
  const a = Date.parse(prev.ts);
  const b = Date.parse(evt.ts);
  if (Number.isNaN(a) || Number.isNaN(b)) return null;
  const d = b - a;
  if (d < 1000) return `+${d}ms`;
  return `+${(d / 1000).toFixed(1)}s`;
}

function rawOf(evt: TraceEvent): string {
  const rest: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(evt)) {
    if (k === "seq" || k === "ts" || k === "kind") continue;
    rest[k] = v;
  }
  return JSON.stringify(rest, null, 2);
}

/**
 * Render one event as `icon · label · detail · Δ`, tone-classed. Never
 * throws: an unknown kind falls back to a titled label so a
 * forward-compatible trace still reads.
 */
export function formatEvent(
  evt: TraceEvent,
  prev?: TraceEvent,
): FormattedEvent {
  const delta = relTime(evt, prev);
  const raw = rawOf(evt);
  const base = { delta, raw };

  switch (evt.kind) {
    case "step_started":
    case "setup_step_started": {
      const uses = str(evt.uses) ?? "step";
      const layer = str(evt.layer);
      const args = describeWith(evt.with);
      const detail = [layer, args].filter(Boolean).join(" · ");
      return { ...base, icon: "▶", label: actionVerb(uses), detail, tone: "muted" };
    }
    case "step_observation":
    case "setup_step_observation": {
      const name = str(evt.output_name) ?? "output";
      if (typeof evt.blob_sha256 === "string") {
        const { icon, label } = blobLabel(name);
        return { ...base, icon, label, detail: "", tone: "muted", blobSha: evt.blob_sha256 };
      }
      return {
        ...base,
        icon: "·",
        label: "observed",
        detail: `${name} = ${compactValue(evt.value)}`,
        tone: "muted",
      };
    }
    case "step_finished":
    case "setup_step_finished": {
      const outcome = str(evt.outcome) ?? "ok";
      const map: Record<string, { icon: string; label: string; tone: Tone }> = {
        ok: { icon: "✓", label: "step ok", tone: "ok" },
        error: { icon: "✗", label: "step error", tone: "fail" },
        timeout: { icon: "⏱", label: "step timed out", tone: "inconclusive" },
      };
      const m = map[outcome] ?? { icon: "·", label: `step ${outcome}`, tone: "muted" as Tone };
      return { ...base, ...m, detail: "" };
    }
    case "assertion_evaluated": {
      const state = str(evt.state) ?? "";
      const tone = toneOfState(state);
      const detail = str(evt.detail) ?? "";
      if (state === "pass") {
        return { ...base, icon: "✓", label: "assertion held", detail, tone };
      }
      const label = state.startsWith("inconclusive")
        ? "assertion inconclusive"
        : "assertion failed";
      return { ...base, icon: "✗", label, detail, tone };
    }
    case "check_finished": {
      const verdict = str(evt.verdict) ?? "";
      const pass = verdict === "pass";
      return {
        ...base,
        icon: pass ? "✓" : "⛔",
        label: `verdict: ${verdict}`,
        detail: "",
        tone: "anchor",
      };
    }
    case "run_started":
      return { ...base, icon: "▶", label: "run started", detail: str(evt.verification_path) ?? "", tone: "muted" };
    default:
      return { ...base, icon: "·", label: evt.kind.replace(/_/g, " "), detail: "", tone: "muted" };
  }
}

export interface CheckSummaryModel {
  verdict: CheckDetail["verdict"];
  /** Plain-language "what happened" line. */
  headline: string;
  /** For a non-pass, the recorded failing-assertion detail lines. */
  failing: string[];
}

/**
 * Derive a plain-language check summary from the recorded timeline —
 * mechanical, never recomputed. States the verdict and, for a
 * non-pass, surfaces each non-passing assertion's recorded cause.
 */
export function summarizeCheck(detail: CheckDetail): CheckSummaryModel {
  const assertions = detail.timeline.filter((e) => e.kind === "assertion_evaluated");
  const failing = assertions
    .filter((e) => str(e.state) !== "pass")
    .map((e) => str(e.detail) || str(e.state) || "no detail recorded");

  const v = detail.verdict;
  let headline: string;
  if (v === "pass") {
    const n = assertions.length;
    headline = `This check passed — ${n} assertion${n === 1 ? "" : "s"} held against the live delivery web.`;
  } else if (v === "fail") {
    headline =
      failing.length === 1
        ? "This check failed — an assertion did not hold:"
        : `This check failed — ${failing.length} assertions did not hold:`;
  } else if (v && v.startsWith("inconclusive")) {
    headline = `This check was inconclusive (${v.slice("inconclusive:".length) || "unknown cause"}) — it could not be decided:`;
  } else {
    headline = "This check has no recorded verdict yet.";
  }
  return { verdict: v, headline, failing };
}
