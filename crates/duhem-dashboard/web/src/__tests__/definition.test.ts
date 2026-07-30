import { describe, expect, it } from "vitest";
import { parseDefinition } from "../definition";

describe("parseDefinition step labels", () => {
  it("exposes descriptions without changing id-based navigation keys", () => {
    const vd = parseDefinition(`
criteria:
  - id: AC-1
    checks:
      - id: AC-1.1
        steps:
          - id: submit
            description: Submit the sign-in form
            uses: ui/click
`);
    expect(vd.stepLabel("AC-1", "AC-1.1", 0)).toBe("Submit the sign-in form");
    expect(vd.stepId("AC-1", "AC-1.1", 0)).toBe("submit");
  });

  it("falls back through id to action and index for older snapshots", () => {
    const vd = parseDefinition(`
criteria:
  - id: AC-1
    checks:
      - id: AC-1.1
        steps:
          - id: submit
            uses: ui/click
          - uses: ui/assert-element
`);
    expect(vd.stepLabel("AC-1", "AC-1.1", 0)).toBe("submit");
    expect(vd.stepLabel("AC-1", "AC-1.1", 1)).toBe("ui/assert-element #1");
    expect(vd.stepLabel("AC-1", "AC-1.1", 2)).toBeUndefined();
  });
});
