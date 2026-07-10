// Unit tests for the pure event/summary formatters (#206).

import { describe, expect, it } from "vitest";
import { describeWith, formatEvent, summarizeCheck } from "../format";
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
    expect(f.icon).toBe("▶");
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
    expect(f.icon).toBe("📷");
    expect(f.label).toBe("screenshot captured");
    expect(f.blobSha).toBe("abc123");
  });

  it("tones step_finished by outcome", () => {
    expect(formatEvent(ev("step_finished", { outcome: "ok" })).tone).toBe("ok");
    expect(formatEvent(ev("step_finished", { outcome: "error" })).tone).toBe("fail");
    expect(formatEvent(ev("step_finished", { outcome: "timeout" })).tone).toBe("inconclusive");
  });

  it("emphasizes a failing assertion with its recorded detail", () => {
    const f = formatEvent(ev("assertion_evaluated", { state: "fail", detail: "actual false, expected true" }));
    expect(f.icon).toBe("✗");
    expect(f.label).toBe("assertion failed");
    expect(f.detail).toBe("actual false, expected true");
    expect(f.tone).toBe("fail");
  });

  it("labels an inconclusive assertion distinctly", () => {
    const f = formatEvent(ev("assertion_evaluated", { state: "inconclusive:missing_observation", detail: "missing_observation(x)" }));
    expect(f.label).toBe("assertion inconclusive");
    expect(f.tone).toBe("inconclusive");
  });

  it("anchors the final verdict", () => {
    const fail = formatEvent(ev("check_finished", { verdict: "fail" }));
    expect(fail.icon).toBe("⛔");
    expect(fail.label).toBe("verdict: fail");
    expect(fail.tone).toBe("anchor");
    expect(formatEvent(ev("check_finished", { verdict: "pass" })).icon).toBe("✓");
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
