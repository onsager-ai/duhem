// Unit tests for the pure event/summary formatters (#206).

import { describe, expect, it } from "vitest";
import {
  assertionText,
  describeWith,
  formatEvent,
  groupTimeline,
  stepStatus,
  summarizeCheck,
} from "../format";
import type { StepNode } from "../format";
import type { CheckDetail, TraceEvent } from "../api";

const ev = (kind: string, extra: Record<string, unknown> = {}, seq = 1): TraceEvent => ({
  seq,
  ts: "2026-01-01T00:00:00.000Z",
  kind,
  ...extra,
});

describe("formatEvent", () => {
  it("renders a navigate step as a verb + url, muted", () => {
    const f = formatEvent(ev("step_started", { uses: "ui/navigate", layer: "ui", with: { url: "http://x/" } }));
    expect(f.icon).toBe("action");
    expect(f.label).toBe("navigate");
    expect(f.detail).toBe("ui · http://x/");
    expect(f.tone).toBe("muted");
  });

  it("renders an assert-element step with a readable locator", () => {
    const f = formatEvent(
      ev("step_started", {
        uses: "ui/assert-element",
        layer: "ui",
        with: { locator: { role: "status", text: "Payment complete" }, expected: "visible", within: "4s" },
      }),
    );
    expect(f.label).toBe("assert-element");
    expect(f.detail).toBe('ui · role=status, text "Payment complete" · visible · within 4s');
  });

  it("renders an inline observation as name = value", () => {
    const f = formatEvent(ev("step_observation", { output_name: "satisfied", value: false }));
    expect(f.label).toBe("observed");
    expect(f.detail).toBe("satisfied = false");
    expect(f.blobSha).toBeUndefined();
  });

  it("renders a capture blob observation with a friendly label + sha", () => {
    const f = formatEvent(ev("step_observation", { output_name: "capture/screenshot", blob_sha256: "abc123" }));
    expect(f.icon).toBe("screenshot");
    expect(f.label).toBe("screenshot captured");
    expect(f.blobSha).toBe("abc123");
  });

  it("labels a capture/video observation (#215)", () => {
    const f = formatEvent(ev("step_observation", { output_name: "capture/video", blob_sha256: "vid1" }));
    expect(f.icon).toBe("video");
    expect(f.label).toBe("video recorded");
    expect(f.blobSha).toBe("vid1");
  });

  it("tones step_finished by outcome", () => {
    expect(formatEvent(ev("step_finished", { outcome: "ok" })).tone).toBe("ok");
    expect(formatEvent(ev("step_finished", { outcome: "error" })).tone).toBe("fail");
    expect(formatEvent(ev("step_finished", { outcome: "timeout" })).tone).toBe("inconclusive");
  });

  it("emphasizes a failing assertion with its recorded detail", () => {
    const f = formatEvent(ev("assertion_evaluated", { state: "fail", detail: "actual false, expected true" }));
    expect(f.icon).toBe("fail");
    expect(f.label).toBe("assertion failed");
    expect(f.detail).toBe("actual false, expected true");
    expect(f.tone).toBe("fail");
  });

  it("labels an inconclusive assertion distinctly", () => {
    const f = formatEvent(ev("assertion_evaluated", { state: "inconclusive:missing_observation", detail: "missing_observation(x)" }));
    expect(f.label).toBe("assertion inconclusive");
    expect(f.tone).toBe("inconclusive");
  });

  it("does not mislabel a missing or unknown assertion state as failed", () => {
    const missing = formatEvent(ev("assertion_evaluated", { detail: "x" }));
    expect(missing.label).toBe("assertion evaluated");
    expect(missing.tone).toBe("muted");
    const future = formatEvent(ev("assertion_evaluated", { state: "skipped" }));
    expect(future.label).toBe("assertion evaluated");
    expect(future.tone).toBe("muted");
  });

  it("anchors the final verdict", () => {
    const fail = formatEvent(ev("check_finished", { verdict: "fail" }));
    expect(fail.icon).toBe("verdict-fail");
    expect(fail.label).toBe("verdict: fail");
    expect(fail.tone).toBe("anchor");
    expect(formatEvent(ev("check_finished", { verdict: "pass" })).icon).toBe("verdict-pass");
  });

  it("falls back to a titled label for an unknown kind, never throwing", () => {
    const f = formatEvent(ev("some_future_event", { whatever: 1 }));
    expect(f.label).toBe("some future event");
    expect(f.tone).toBe("muted");
  });

  it("computes a relative delta from the previous event and preserves raw", () => {
    const prev = ev("step_started", { uses: "ui/navigate" }, 1);
    const cur: TraceEvent = { ...ev("step_finished", { outcome: "ok" }, 2), ts: "2026-01-01T00:00:04.000Z" };
    const f = formatEvent(cur, prev);
    expect(f.delta).toBe("+4.0s");
    expect(f.raw).toContain('"outcome": "ok"');
    expect(f.raw).not.toContain('"seq"');
  });
});

describe("describeWith", () => {
  it("returns empty for a missing payload", () => {
    expect(describeWith(undefined)).toBe("");
  });
  it("falls back to scalar key=value pairs for an unknown action shape", () => {
    expect(describeWith({ foo: "bar", n: 3 })).toBe("foo=bar · n=3");
  });
});

const check = (verdict: CheckDetail["verdict"], timeline: TraceEvent[]): CheckDetail => ({
  criterion_id: "AC-1",
  check_id: "AC-1.1",
  verdict,
  spans: [],
  timeline,
  artifacts: [],
});

describe("groupTimeline", () => {
  it("folds a step's lifecycle + its trailing capture into one node; check-level events standalone", () => {
    const events: TraceEvent[] = [
      ev("step_started", { step_index: 0, uses: "ui/navigate" }, 1),
      ev("step_finished", { step_index: 0, outcome: "ok" }, 2),
      ev("step_started", { step_index: 1, uses: "ui/assert-element" }, 3),
      ev("step_observation", { step_index: 1, output_name: "satisfied", value: false }, 4),
      ev("step_finished", { step_index: 1, outcome: "ok" }, 5),
      ev("assertion_evaluated", { state: "fail", detail: "x" }, 6),
      // A capture blob observation is emitted after the step closed, but
      // carries a step_index → it nests back onto step 1 (#280 polish).
      ev("step_observation", { step_index: 1, output_name: "capture/screenshot", blob_sha256: "abc" }, 7),
      ev("check_finished", { verdict: "fail" }, 8),
    ];
    const nodes = groupTimeline(events);
    // The capture (seq 7) folds onto step 1; the assertion (no
    // step_index) and the verdict stay standalone.
    expect(nodes.map((n) => n.kind)).toEqual(["step", "step", "event", "event"]);
    const step1 = nodes[1];
    if (step1.kind !== "step") throw new Error("expected step");
    expect(step1.stepIndex).toBe(1);
    expect(step1.events.map((e) => e.seq)).toEqual([3, 4, 5, 7]);
    // The first standalone event is the un-indexed assertion, not a capture.
    if (nodes[2].kind !== "event") throw new Error("expected event");
    expect(nodes[2].event.seq).toBe(6);
  });

  it("returns a flat list when there are no steps", () => {
    const nodes = groupTimeline([ev("check_finished", { verdict: "pass" })]);
    expect(nodes).toHaveLength(1);
    expect(nodes[0].kind).toBe("event");
  });

  it("folds an implicit judgment (step_index) into its step, not an orphan row (#280)", () => {
    const events: TraceEvent[] = [
      ev("step_started", { step_index: 1, uses: "ui/assert-element" }, 3),
      ev("step_observation", { step_index: 1, output_name: "satisfied", value: false }, 4),
      ev("step_observation", { step_index: 1, output_name: "count", value: 1 }, 5),
      ev("step_finished", { step_index: 1, outcome: "ok" }, 6),
      ev(
        "assertion_evaluated",
        { state: "fail", detail: 'expected text "Manager" to be absent', step_index: 1 },
        7,
      ),
      ev("check_finished", { verdict: "fail" }, 8),
    ];
    const nodes = groupTimeline(events);
    // The assertion is folded away — only the step and the verdict remain.
    expect(nodes.map((n) => n.kind)).toEqual(["step", "event"]);
    const step = nodes[0];
    if (step.kind !== "step") throw new Error("expected step");
    expect(step.judgment?.seq).toBe(7);
    // …but stays reachable inside the step (its raw is one click away).
    expect(step.events.some((e) => e.seq === 7)).toBe(true);
  });

  it("keeps an explicit assertion (no step_index) standalone (#280)", () => {
    const events: TraceEvent[] = [
      ev("step_started", { step_index: 0, uses: "api/call" }, 1),
      ev("step_finished", { step_index: 0, outcome: "ok" }, 2),
      ev("assertion_evaluated", { state: "fail", detail: "actual 500, expected 200" }, 3),
    ];
    const nodes = groupTimeline(events);
    expect(nodes.map((n) => n.kind)).toEqual(["step", "event"]);
    const step = nodes[0];
    if (step.kind !== "step") throw new Error("expected step");
    expect(step.judgment).toBeUndefined();
  });
});

describe("stepStatus (#280 status propagation)", () => {
  const node = (judgment?: TraceEvent, outcome = "ok"): StepNode => ({
    kind: "step",
    key: "s1",
    stepIndex: 0,
    events: [
      ev("step_started", { step_index: 0, uses: "ui/assert-element" }, 1),
      ev("step_finished", { step_index: 0, outcome }, 2),
    ],
    judgment,
  });

  it("propagates a failed judgment to a failed step, carrying the reason", () => {
    const s = stepStatus(
      node(
        ev("assertion_evaluated", {
          state: "fail",
          detail: 'expected text "Manager" to be absent within 5s, but 1 still matched',
          step_index: 0,
        }),
      ),
    );
    expect(s.label).toBe("step failed");
    expect(s.tone).toBe("fail");
    expect(s.failed).toBe(true);
    expect(s.reason).toContain("Manager");
  });

  it("keeps a passing judging step green with no reason", () => {
    const s = stepStatus(node(ev("assertion_evaluated", { state: "pass", step_index: 0 })));
    expect(s.label).toBe("step ok");
    expect(s.tone).toBe("ok");
    expect(s.failed).toBe(false);
    expect(s.reason).toBe("");
  });

  it("propagates an inconclusive judgment", () => {
    const s = stepStatus(
      node(ev("assertion_evaluated", { state: "inconclusive:missing_observation", step_index: 0 })),
    );
    expect(s.label).toBe("step inconclusive");
    expect(s.tone).toBe("inconclusive");
    expect(s.failed).toBe(true);
  });

  it("falls back to the step_finished outcome with no judgment", () => {
    expect(stepStatus(node(undefined, "ok")).label).toBe("step ok");
    expect(stepStatus(node(undefined, "error")).label).toBe("step error");
    expect(stepStatus(node(undefined, "error")).tone).toBe("fail");
  });
});

describe("assertionText (#279 follow-up)", () => {
  it("combines the authored expression and the observed detail", () => {
    expect(
      assertionText(
        ev("assertion_evaluated", {
          expr: "$steps.update.outputs.status == 200",
          detail: "actual 500, expected 200",
        }),
      ),
    ).toBe("$steps.update.outputs.status == 200 — actual 500, expected 200");
  });
  it("falls back to detail alone (an implicit judgment has no authored line)", () => {
    expect(assertionText(ev("assertion_evaluated", { detail: 'expected text "Manager" to be absent' }))).toBe(
      'expected text "Manager" to be absent',
    );
  });
  it("falls back to expr alone (a passing explicit assertion has no detail)", () => {
    expect(assertionText(ev("assertion_evaluated", { expr: "$steps.q.outputs.status == 200" }))).toBe(
      "$steps.q.outputs.status == 200",
    );
  });
});

describe("explicit assertion folds onto its step (#279 follow-up)", () => {
  it("paints an api step red and shows the expression as the reason", () => {
    const events: TraceEvent[] = [
      ev("step_started", { step_index: 0, uses: "api/call" }, 1),
      ev("step_observation", { step_index: 0, output_name: "status", value: 500 }, 2),
      ev("step_finished", { step_index: 0, outcome: "ok" }, 3),
      ev(
        "assertion_evaluated",
        {
          state: "fail",
          expr: "$steps.update.outputs.status == 200",
          detail: "actual 500, expected 200",
          step_index: 0,
        },
        4,
      ),
      ev("check_finished", { verdict: "fail" }, 5),
    ];
    const nodes = groupTimeline(events);
    // The assertion folded away; only the step + verdict remain at top.
    expect(nodes.map((n) => n.kind)).toEqual(["step", "event"]);
    const step = nodes[0];
    if (step.kind !== "step") throw new Error("expected step");
    const s = stepStatus(step);
    // The api call "ran" (step ok) but its assertion failed → step failed.
    expect(s.label).toBe("step failed");
    expect(s.tone).toBe("fail");
    expect(s.reason).toBe("$steps.update.outputs.status == 200 — actual 500, expected 200");
  });
});

describe("summarizeCheck", () => {
  it("states a pass with the assertion count", () => {
    const s = summarizeCheck(
      check("pass", [ev("assertion_evaluated", { state: "pass" }), ev("assertion_evaluated", { state: "pass" }, 2)]),
    );
    expect(s.headline).toContain("passed");
    expect(s.headline).toContain("2 assertions held");
    expect(s.failing).toEqual([]);
  });

  it("surfaces the failing assertion's recorded cause on a fail", () => {
    const s = summarizeCheck(
      check("fail", [ev("assertion_evaluated", { state: "fail", detail: "actual false, expected true" })]),
    );
    expect(s.headline).toContain("failed");
    expect(s.failing).toEqual(["actual false, expected true"]);
  });

  it("names the cause on an inconclusive", () => {
    const s = summarizeCheck(check("inconclusive:environment_error", []));
    expect(s.headline).toContain("inconclusive");
    expect(s.headline).toContain("environment_error");
  });
});
