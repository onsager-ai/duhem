// #560: the brand mark and the Appendix B verdict mark.
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it } from "vitest";

import { VerdictMark } from "../components/brand/VerdictMark";
import { BrandMark } from "../components/layout/BrandMark";

afterEach(cleanup);

const center = (svg: Element) => svg.querySelectorAll("rect")[4];

describe("BrandMark", () => {
  it("keeps the heading's accessible name to the wordmark", () => {
    render(
      <MemoryRouter>
        <BrandMark asHeading />
      </MemoryRouter>,
    );
    // The self-verification VD locates { role: heading, name: Duhem }.
    expect(screen.getByRole("heading", { level: 1, name: "Duhem" })).toBeTruthy();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("renders the §1 geometry in currentColor with no plate", () => {
    const { container } = render(
      <MemoryRouter>
        <BrandMark />
      </MemoryRouter>,
    );
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("viewBox")).toBe("0 0 32 32");
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    const rects = [...svg.querySelectorAll("rect")].map((r) =>
      ["x", "y", "width", "height"].map((a) => Number(r.getAttribute(a))),
    );
    expect(rects).toEqual([
      [2, 2, 28, 4],
      [2, 26, 28, 4],
      [2, 6, 4, 20],
      [26, 6, 4, 20],
      [11, 11, 10, 10],
    ]);
    expect(svg.parentElement!.className).not.toMatch(/bg-/);
  });
});

describe("VerdictMark", () => {
  it.each([
    ["pass", "fill-pass"],
    ["fail", "fill-fail"],
    ["inconclusive:timeout", "fill-inconclusive"],
  ] as const)("colors only the center for %s", (verdict, cls) => {
    const { container } = render(<VerdictMark verdict={verdict} status="finished" />);
    const svg = container.querySelector("svg")!;
    expect(center(svg).getAttribute("class")).toContain(cls);
    // The frame stays foreground ink.
    expect(svg.querySelector("g")!.getAttribute("fill")).toBe("currentColor");
    expect(svg.getAttribute("class")).toContain("text-foreground");
  });

  it("pulses the center only while running", () => {
    const { container, rerender } = render(<VerdictMark verdict={null} status="running" />);
    expect(center(container.querySelector("svg")!).getAttribute("class")).toContain(
      "verdict-mark-pulse",
    );
    rerender(<VerdictMark verdict="pass" status="finished" />);
    expect(center(container.querySelector("svg")!).getAttribute("class")).not.toContain(
      "verdict-mark-pulse",
    );
  });

  it("is decorative by default and an image only when labelled", () => {
    const { rerender } = render(<VerdictMark verdict="fail" />);
    expect(screen.queryByRole("img")).toBeNull();
    rerender(<VerdictMark verdict="fail" label="Verdict: fail" />);
    expect(screen.getByRole("img", { name: "Verdict: fail" })).toBeTruthy();
  });
});
