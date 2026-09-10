import { expect, it } from "vitest";
import type { TraceEvent } from "../api";
import { groupTimeline, stepStatus } from "../format";
import { groupLoops, stepNavigation } from "../step-presentation";

it("groups inner steps by name, invocation and iteration, even with interleaved flow bodies", () => {
  const events: TraceEvent[] = [];
  for (let iteration = 0; iteration < 50; iteration++) {
    for (const name of ["first", "second"]) {
      for (let inner_index = 0; inner_index < 2; inner_index++) {
        events.push({ seq: events.length, ts: "t", kind: "setup_step_started", step_index: 4,
          flow: { name, invocation: "loop", iteration, inner_index } });
        events.push({ seq: events.length, ts: "t", kind: "setup_step_finished", step_index: 4, outcome: "ok" });
      }
    }
  }
  const nodes = groupTimeline(events);
  const groups = groupLoops(nodes);
  expect(groups).toHaveLength(2);
  for (const group of groups) {
    expect(group.kind).toBe("loop");
    if (group.kind !== "loop") throw new Error("expected loop");
    expect(group.iterations).toHaveLength(50);
    expect(group.iterations[37].steps).toHaveLength(2);
    expect(group.iterations[37].steps.map((step) => step.stepIndex)).toEqual([4, 4]);
  }
  const keys = nodes.flatMap((node) => node.kind === "step" ? [stepNavigation(node).key] : []);
  expect(new Set(keys).size).toBe(200);
});

it("attaches trailing judgments and captures to the preceding iteration, not the last shared index", () => {
  const nodes = groupTimeline(Array.from({ length: 3 }, (_, iteration): TraceEvent[] => [
    { seq: iteration * 4, ts: "t", kind: "step_started", step_index: 1,
      flow: { name: "for_each", invocation: "loop", inner_index: 0, iteration } },
    { seq: iteration * 4 + 1, ts: "t", kind: "step_finished", step_index: 1, outcome: "ok" },
    { seq: iteration * 4 + 2, ts: "t", kind: "assertion_evaluated", step_index: 1, state: iteration === 1 ? "fail" : "pass" },
    { seq: iteration * 4 + 3, ts: "t", kind: "step_observation", step_index: 1, output_name: "capture/screenshot", blob_sha256: `frame-${iteration}` },
  ]).flat());
  expect(nodes).toHaveLength(3);
  nodes.forEach((node, iteration) => {
    if (node.kind !== "step") throw new Error("expected step");
    expect(stepStatus(node).failed).toBe(iteration === 1);
    expect(node.events.filter((event) => event.blob_sha256).map((event) => event.blob_sha256))
      .toEqual([`frame-${iteration}`]);
  });
});

it("keeps the recorded loop gate inside a multi-iteration construct", () => {
  const trace: TraceEvent[] = [
    { seq: 0, ts: "t", kind: "setup_step_started", step_index: 1, phase: "setup", uses: "for_each" },
    { seq: 1, ts: "t", kind: "setup_step_finished", step_index: 1, outcome: "ok" },
    ...Array.from({ length: 50 }, (_, iteration): TraceEvent[] => [
      { seq: 2 + iteration * 2, ts: "t", kind: "setup_step_started", phase: "setup", step_index: 1,
        flow: { name: "for_each", invocation: "loop", inner_index: 0, iteration } },
      { seq: 3 + iteration * 2, ts: "t", kind: "setup_step_finished", step_index: 1, outcome: "ok" },
    ]).flat(),
    { seq: 102, ts: "t", kind: "setup_step_started", phase: "setup", step_index: 2, uses: "cli/invoke" },
    { seq: 103, ts: "t", kind: "setup_step_finished", step_index: 2, outcome: "ok" },
  ];
  const grouped = groupLoops(groupTimeline(trace));
  expect(grouped).toHaveLength(2);
  const loop = grouped[0];
  if (loop.kind !== "loop") throw new Error("expected loop");
  expect(loop.preamble?.events.map((event) => event.seq)).toEqual([0, 1]);
  expect(loop.iterations).toHaveLength(50);
  expect(grouped[1]).toMatchObject({ kind: "step", stepIndex: 2 });
  // A loop gate with no expanded evidence (empty array or failed gate) stays
  // visible as its recorded step, with no invented iterations or chrome.
  expect(groupLoops(groupTimeline(trace.slice(0, 2)))).toMatchObject([{ kind: "step", stepIndex: 1 }]);
});
