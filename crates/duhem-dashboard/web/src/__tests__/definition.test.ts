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

  it("parses authored parameters and tolerates absent or malformed maps", () => {
    const vd = parseDefinition(`
criteria:
  - id: AC-1
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/click
            with:
              locator: $pages.login.submit
          - uses: ui/wait
          - uses: ui/navigate
            with: malformed
`);

    expect(vd.stepWith("AC-1", "AC-1.1", 0)).toEqual({
      locator: "$pages.login.submit",
    });
    expect(vd.stepWith("AC-1", "AC-1.1", 1)).toBeUndefined();
    expect(vd.stepWith("AC-1", "AC-1.1", 2)).toBeUndefined();
    expect(vd.stepWith("missing", "missing", 0)).toBeUndefined();
  });

  it("joins expanded steps through flow provenance and keeps index fallback", () => {
    const vd = parseDefinition(`
flows:
  sign_in:
    description: Authenticate with supplied credentials
    steps:
      - id: user
        description: Enter the username
        uses: ui/type
        with:
          locator: $pages.login.user
      - uses: ui/click
  undescribed:
    steps:
      - uses: ui/click
criteria:
  - id: AC-1
    checks:
      - id: AC-1.1
        steps:
          - id: login
            description: Sign in as the fixture user
            call: sign_in
          - id: legacy
            uses: ui/assert-url
          - id: fallback_description
            description: Sign in at this call site
            call: undescribed
          - id: fallback_id
            call: undescribed
`);
    const flow = { name: "sign_in", invocation: "login", inner_index: 0 };
    expect(vd.flowLabel("AC-1", "AC-1.1", flow))
      .toBe("Authenticate with supplied credentials");
    expect(vd.stepLabel("AC-1", "AC-1.1", 0, flow))
      .toBe("Sign in as the fixture user › Enter the username");
    expect(vd.flowStepLabel(flow)).toBe("Enter the username");
    expect(vd.stepId("AC-1", "AC-1.1", 0, flow)).toBe("login__user");
    expect(vd.stepWith("AC-1", "AC-1.1", 0, flow)).toEqual({
      locator: "$pages.login.user",
    });
    expect(vd.stepLabel("AC-1", "AC-1.1", 1)).toBe("legacy");

    const describedCall = {
      name: "undescribed",
      invocation: "fallback_description",
      inner_index: 0,
    };
    expect(vd.flowLabel("AC-1", "AC-1.1", describedCall))
      .toBe("Sign in at this call site");
    const identifiedCall = {
      name: "undescribed",
      invocation: "fallback_id",
      inner_index: 0,
    };
    expect(vd.flowLabel("AC-1", "AC-1.1", identifiedCall)).toBe("fallback_id");
    const unmatchedCall = {
      name: "undescribed",
      invocation: "raw_invocation",
      inner_index: 0,
    };
    expect(vd.flowLabel("AC-1", "AC-1.1", unmatchedCall)).toBe("raw_invocation");
  });
});
