// Human-readable rendering of trace events (#206). Pure functions over
// the wire-format `TraceEvent` — the check page's timeline and summary
// are derived here, never re-judged and never LLM-authored. The raw
// JSON stays one click away in the UI; this is the legible default.

import type { CheckDetail, TraceEvent } from "./api";

export type Tone = "ok" | "fail" | "inconclusive" | "muted" | "anchor";

/**
 * A semantic icon key, not a glyph. `format.ts` stays pure (no JSX):
 * it names *what the row is* and the view layer (`EventIcon`) maps each
 * key to a lucide SVG. This replaced the former inline emoji (#284
 * follow-up) so every timeline icon is a real, theme-aware vector.
 */
export type IconName =
  | "action" // a step / the run beginning
  | "observed" // a scalar output pulled from the web
  | "pass" // a step ok / an assertion held
  | "fail" // a step error / an assertion failed
  | "inconclusive" // an assertion that could not be decided
  | "timeout" // a step that timed out
  | "verdict-pass" // the check's final pass verdict
  | "verdict-fail" // the check's final fail verdict
  | "screenshot"
  | "dom"
  | "network"
  | "target"
  | "video"
  | "attachment"
  | "unknown"; // forward-compatible fallback

export interface FormattedEvent {
  icon: IconName;
  label: string;
  detail: string;
  tone: Tone;
  /** `+4.0s` relative to the previous event, or null for the first. */
  delta: string | null;
  /** Pretty payload (minus seq/ts/kind) for the row's raw toggle. */
  raw: string;
  /** Set for a blob observation so the row can link the artifact. */
  blobSha?: string;
  /** The assertion expression this outcome evaluated (`assertion_evaluated`
   *  events only) — *what was asserted*, distinct from the observed
   *  operands in `detail`. Absent on runs recorded before #284. */
  expr?: string;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}

/**
 * The human line for an assertion outcome — the authored expression AND
 * the observed-vs-expected detail (#279 follow-up), e.g.
 * `$steps.update.outputs.status == 200 — actual 500, expected 200`. Shows
 * *what* was asserted, not just the values. Falls back to whichever is
 * present (implicit judgments carry only a semantic `detail`; a passing
 * explicit assertion carries only the `expr`).
 */
export function assertionText(evt: TraceEvent): string {
  const expr = str(evt.expr);
  const detail = str(evt.detail);
  if (expr && detail) return `${expr} — ${detail}`;
  return detail || expr || "";
}

/** Compact one-line value for an inline observation or arg scalar. */
export function compactValue(v: unknown): string {
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
function blobLabel(outputName: string): { icon: IconName; label: string } {
  switch (outputName) {
    case "capture/screenshot":
      return { icon: "screenshot", label: "screenshot captured" };
    case "capture/dom":
      return { icon: "dom", label: "DOM captured" };
    case "capture/network":
      return { icon: "network", label: "network captured" };
    case "capture/target-rect":
      return { icon: "target", label: "target highlighted" };
    case "capture/video":
      return { icon: "video", label: "video recorded" };
    default:
      return { icon: "attachment", label: `${outputName} captured` };
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
      return { ...base, icon: "action", label: actionVerb(uses), detail, tone: "muted" };
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
        icon: "observed",
        label: "observed",
        detail: `${name} = ${compactValue(evt.value)}`,
        tone: "muted",
      };
    }
    case "step_finished":
    case "setup_step_finished": {
      const outcome = str(evt.outcome) ?? "ok";
      const map: Record<string, { icon: IconName; label: string; tone: Tone }> = {
        ok: { icon: "pass", label: "step ok", tone: "ok" },
        error: { icon: "fail", label: "step error", tone: "fail" },
        timeout: { icon: "timeout", label: "step timed out", tone: "inconclusive" },
      };
      const m = map[outcome] ?? { icon: "unknown" as IconName, label: `step ${outcome}`, tone: "muted" as Tone };
      return { ...base, ...m, detail: "" };
    }
    case "assertion_evaluated": {
      const state = str(evt.state) ?? "";
      const detail = str(evt.detail) ?? "";
      // `expr` (the human-authored rule) rides the raw event since #284;
      // it is `undefined` on older runs, and the row falls back to the
      // operands alone.
      const ab = { ...base, expr: str(evt.expr) };
      if (state === "pass") {
        return { ...ab, icon: "pass", label: "assertion held", detail, tone: "ok" };
      }
      if (state === "fail") {
        return { ...ab, icon: "fail", label: "assertion failed", detail, tone: "fail" };
      }
      if (state.startsWith("inconclusive")) {
        return { ...ab, icon: "inconclusive", label: "assertion inconclusive", detail, tone: "inconclusive" };
      }
      // Missing or unknown future state — record it, don't call it a
      // failure (a forward-compatible trace must not be mislabeled).
      return { ...ab, icon: "unknown", label: "assertion evaluated", detail: detail || state, tone: "muted" };
    }
    case "check_finished": {
      const verdict = str(evt.verdict) ?? "";
      const pass = verdict === "pass";
      return {
        ...base,
        icon: pass ? "verdict-pass" : "verdict-fail",
        label: `verdict: ${verdict}`,
        detail: "",
        tone: "anchor",
      };
    }
    case "run_started":
      return { ...base, icon: "action", label: "run started", detail: str(evt.verification_path) ?? "", tone: "muted" };
    default:
      return { ...base, icon: "unknown", label: evt.kind.replace(/_/g, " "), detail: "", tone: "muted" };
  }
}

/**
 * Split a failed-comparison assertion detail — the runtime emits
 * `actual <lhs>, expected <rhs>` for a `false` comparison (see
 * `duhem-runtime` `describe_comparison`) — into its two observed
 * operands, so the check page can render an expected/actual pair
 * instead of a sentence. Returns `null` for any other detail shape (an
 * inconclusive cause, a freeform judgment message), which the caller
 * shows verbatim. The values are already display-formatted by the
 * runtime (strings quoted), so they travel through untouched.
 */
export function parseComparison(
  detail: string | undefined,
): { expected: string; actual: string } | null {
  if (!detail) return null;
  const m = /^actual (.+), expected (.+)$/s.exec(detail);
  if (!m) return null;
  return { actual: m[1], expected: m[2] };
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
    .map((e) => assertionText(e) || str(e.state) || "no detail recorded");

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

/**
 * A step node groups a step's own lifecycle (`step_started` → its
 * observations → `step_finished`) into one collapsible block; every
 * other event (assertions, the verdict, trailing captures) stays a
 * standalone row so the signal is never hidden inside a step.
 *
 * `judgment` is the implicit-judgment `assertion_evaluated` folded back
 * onto this step (#280): its verdict IS the step's `satisfied` outcome,
 * so it propagates the step's status (a failed judgment makes a failed
 * step) instead of floating as an orphan row below. Explicit
 * `assertions:` (no `step_index`) stay standalone.
 */
export type StepNode = {
  kind: "step";
  key: string;
  stepIndex: number;
  events: TraceEvent[];
  judgment?: TraceEvent;
};
export type TimelineNode = StepNode | { kind: "event"; key: string; event: TraceEvent };

const STEP_STARTED = new Set(["step_started", "setup_step_started"]);
const STEP_INNER = new Set([
  "step_observation",
  "setup_step_observation",
  "step_finished",
  "setup_step_finished",
]);

/**
 * Fold the flat event stream into step groups + standalone rows, so
 * the timeline reads as N steps rather than 3N events. Trailing
 * capture observations (emitted after the assertion, once the step
 * group has closed) stay standalone — they read well on their own.
 */
export function groupTimeline(events: TraceEvent[]): TimelineNode[] {
  const nodes: TimelineNode[] = [];
  let group: StepNode | null = null;
  const flush = () => {
    if (group) {
      nodes.push(group);
      group = null;
    }
  };
  for (const evt of events) {
    const si = typeof evt.step_index === "number" ? (evt.step_index as number) : undefined;
    if (STEP_STARTED.has(evt.kind)) {
      flush();
      group = { kind: "step", key: `s${evt.seq}`, stepIndex: si ?? -1, events: [evt] };
      continue;
    }
    if (group && si === group.stepIndex && STEP_INNER.has(evt.kind)) {
      group.events.push(evt);
      if (evt.kind === "step_finished" || evt.kind === "setup_step_finished") flush();
      continue;
    }
    flush();
    nodes.push({ kind: "event", key: `e${evt.seq}`, event: evt });
  }
  flush();
  return foldOntoSteps(nodes);
}

/**
 * Second pass (#280): fold each step-owned event back onto its step node
 * — an `assertion_evaluated` carrying a numeric `step_index` (so the step
 * propagates the verdict, Allure-style), and a trailing `capture/*` blob
 * observation (so its screenshot / DOM / network evidence nests under the
 * step that produced it, not in a side panel). Folded events stay
 * reachable inside the step; an event with no `step_index`, or whose step
 * can't be found, stays a standalone row.
 */
function foldOntoSteps(nodes: TimelineNode[]): TimelineNode[] {
  const stepByIndex = new Map<number, StepNode>();
  for (const n of nodes) if (n.kind === "step") stepByIndex.set(n.stepIndex, n);
  const out: TimelineNode[] = [];
  for (const n of nodes) {
    if (n.kind === "event") {
      const e = n.event;
      if (e.kind === "assertion_evaluated" && typeof e.step_index === "number") {
        const target = stepByIndex.get(e.step_index);
        if (target) {
          // A step may own several assertions (an explicit check can
          // assert two things about one call). A failing one wins for the
          // step's status/reason; otherwise keep the first.
          const prev = target.judgment;
          const isFail = str(e.state) !== "pass";
          if (!prev || (isFail && str(prev.state) === "pass")) {
            target.judgment = e;
          }
          target.events.push(e);
          continue;
        }
      }
      // A trailing capture blob observation nests under its step.
      if (
        (e.kind === "step_observation" || e.kind === "setup_step_observation") &&
        typeof e.blob_sha256 === "string" &&
        typeof e.step_index === "number"
      ) {
        const target = stepByIndex.get(e.step_index);
        if (target) {
          target.events.push(e);
          continue;
        }
      }
    }
    out.push(n);
  }
  return out;
}

/**
 * A step's effective status, propagating a folded implicit judgment
 * (#280). A judging step whose `satisfied` verdict failed is a *failed*
 * step — not a green "step ok" — and carries the semantic reason; the
 * judgment's verdict wins over the action's own "it ran" outcome.
 * Without a failing judgment, falls back to the `step_finished`
 * outcome, exactly as before.
 */
export function stepStatus(node: StepNode): {
  icon: IconName;
  label: string;
  tone: Tone;
  reason: string;
  failed: boolean;
} {
  const jstate = node.judgment ? (str(node.judgment.state) ?? "") : "";
  const reason = node.judgment ? assertionText(node.judgment) : "";
  if (jstate === "fail") {
    return { icon: "fail", label: "step failed", tone: "fail", reason, failed: true };
  }
  if (jstate.startsWith("inconclusive")) {
    return { icon: "inconclusive", label: "step inconclusive", tone: "inconclusive", reason, failed: true };
  }
  const finished = node.events.find(
    (e) => e.kind === "step_finished" || e.kind === "setup_step_finished",
  );
  const outcome = str(finished?.outcome) ?? "ok";
  const map: Record<string, { icon: IconName; label: string; tone: Tone }> = {
    ok: { icon: "pass", label: "step ok", tone: "ok" },
    error: { icon: "fail", label: "step error", tone: "fail" },
    timeout: { icon: "timeout", label: "step timed out", tone: "inconclusive" },
  };
  const m = map[outcome] ?? { icon: "unknown" as IconName, label: `step ${outcome}`, tone: "muted" as Tone };
  return { ...m, reason: "", failed: m.tone === "fail" || m.tone === "inconclusive" };
}
