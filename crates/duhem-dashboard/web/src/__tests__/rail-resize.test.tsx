// #436: the run report rail's draggable/keyboard-resizable splitter
// (`RailSplitter.tsx` + `use-rail-width.ts`). Covers the spec's Test
// section: a keyboard press changes the width, the width stays within
// min/max, persistence survives a remount, and a throwing `localStorage`
// falls back to the default.

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Routes } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { runRoutes } from "../views/run-routes";
import { RAIL_WIDTH_DEFAULT, RAIL_WIDTH_STEP } from "../hooks/use-rail-width";

const STORAGE_KEY = "duhem-rail-width";
const HIDDEN_KEY = "duhem-rail-hidden";

// jsdom does not implement PointerEvent; MouseEvent carries the same
// `clientX` / `button` the splitter's drag handlers read, so it stands
// in cleanly for `fireEvent.pointer*`.
beforeAll(() => {
  if (typeof window.PointerEvent === "undefined") {
    // @ts-expect-error test-only polyfill, see comment above
    window.PointerEvent = window.MouseEvent;
  }
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  try {
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem(HIDDEN_KEY);
  } catch {
    /* ignore */
  }
});

const RUN = {
  run_id: "R1",
  verification: "pegasus-register",
  run_number: 1,
  started_at: "2026-07-22T14:03:00.000Z",
  duration_ms: 1200,
  children: [],
  inputs: {},
  verdict: "fail",
  status: "finished",
  setup_aborted: false,
  has_definition: false,
  criteria: [
    { id: "AC-5", verdict: "fail", checks: [{ id: "AC-5.1", verdict: "fail" }] },
  ],
};

function stub() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify(RUN), { status: 200 })),
  );
}

function renderResults() {
  return render(
    <MemoryRouter initialEntries={["/run/R1/results"]}>
      <Routes>{runRoutes}</Routes>
    </MemoryRouter>,
  );
}

async function findSplitter() {
  return screen.findByRole("separator", { name: "Resize the step list" });
}

describe("run report rail splitter", () => {
  it("renders an accessible vertical separator with value bounds", async () => {
    stub();
    renderResults();
    const splitter = await findSplitter();
    expect(splitter.getAttribute("aria-orientation")).toBe("vertical");
    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));
    expect(splitter.getAttribute("aria-valuemin")).toBe(String(RAIL_WIDTH_DEFAULT));
    expect(Number(splitter.getAttribute("aria-valuemax"))).toBeGreaterThan(
      RAIL_WIDTH_DEFAULT,
    );
    expect(splitter.getAttribute("tabindex")).toBe("0");
    // Hidden below `md:` — the rail and detail pane stack there, so there
    // is nothing to resize (mirrors the existing 42vh mobile rail rule).
    expect(splitter.className).toContain("hidden");
    expect(splitter.className).toContain("md:flex");
  });

  it("ArrowRight/ArrowLeft step the width by 16px and drive the grid's CSS variable", async () => {
    stub();
    const { container } = renderResults();
    const splitter = await findSplitter();
    const grid = container.querySelector(".run-results-grid") as HTMLElement;

    fireEvent.keyDown(splitter, { key: "ArrowRight" });
    expect(splitter.getAttribute("aria-valuenow")).toBe(
      String(RAIL_WIDTH_DEFAULT + RAIL_WIDTH_STEP),
    );
    expect(grid.style.getPropertyValue("--run-rail-width")).toBe(
      `${RAIL_WIDTH_DEFAULT + RAIL_WIDTH_STEP}px`,
    );

    fireEvent.keyDown(splitter, { key: "ArrowLeft" });
    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));
    expect(grid.style.getPropertyValue("--run-rail-width")).toBe(
      `${RAIL_WIDTH_DEFAULT}px`,
    );
  });

  it("clamps ArrowLeft at the minimum (the pre-#436 default)", async () => {
    stub();
    renderResults();
    const splitter = await findSplitter();
    fireEvent.keyDown(splitter, { key: "ArrowLeft" });
    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));
  });

  it("Home and End jump to the min and max", async () => {
    stub();
    renderResults();
    const splitter = await findSplitter();
    const max = splitter.getAttribute("aria-valuemax");

    fireEvent.keyDown(splitter, { key: "End" });
    expect(splitter.getAttribute("aria-valuenow")).toBe(max);

    fireEvent.keyDown(splitter, { key: "Home" });
    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));
  });

  it("clamps a drag past the max to ~60% of the viewport", async () => {
    vi.stubGlobal("innerWidth", 1000);
    stub();
    renderResults();
    const splitter = await findSplitter();
    const expectedMax = Math.round(1000 * 0.6);
    expect(splitter.getAttribute("aria-valuemax")).toBe(String(expectedMax));

    fireEvent.pointerDown(splitter, { clientX: 0, button: 0 });
    fireEvent.pointerMove(window, { clientX: 5000 });
    fireEvent.pointerUp(window);

    expect(splitter.getAttribute("aria-valuenow")).toBe(String(expectedMax));
  });

  it("clamps a drag past the minimum back to the default", async () => {
    stub();
    renderResults();
    const splitter = await findSplitter();

    fireEvent.pointerDown(splitter, { clientX: 500, button: 0 });
    fireEvent.pointerMove(window, { clientX: -5000 });
    fireEvent.pointerUp(window);

    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));
  });

  it("double-click resets a dragged width back to the default", async () => {
    stub();
    renderResults();
    const splitter = await findSplitter();

    fireEvent.keyDown(splitter, { key: "End" });
    expect(splitter.getAttribute("aria-valuenow")).not.toBe(String(RAIL_WIDTH_DEFAULT));

    fireEvent.doubleClick(splitter);
    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));
  });

  it("persists a resized width across a remount", async () => {
    stub();
    const first = renderResults();
    const splitter = await findSplitter();
    fireEvent.keyDown(splitter, { key: "ArrowRight" });
    fireEvent.keyDown(splitter, { key: "ArrowRight" });
    const resized = splitter.getAttribute("aria-valuenow");
    expect(resized).toBe(String(RAIL_WIDTH_DEFAULT + RAIL_WIDTH_STEP * 2));
    first.unmount();

    stub();
    renderResults();
    const remounted = await findSplitter();
    expect(remounted.getAttribute("aria-valuenow")).toBe(resized);
  });

  it("falls back to the default when localStorage throws", async () => {
    const boom = () => {
      throw new Error("storage disabled");
    };
    vi.stubGlobal("localStorage", {
      getItem: boom,
      setItem: boom,
      removeItem: boom,
    });
    stub();
    renderResults();
    const splitter = await findSplitter();
    expect(splitter.getAttribute("aria-valuenow")).toBe(String(RAIL_WIDTH_DEFAULT));

    // Interacting still works in-memory even though persistence throws on
    // every call — the throw must not surface as an uncaught error.
    expect(() => fireEvent.keyDown(splitter, { key: "ArrowRight" })).not.toThrow();
    expect(splitter.getAttribute("aria-valuenow")).toBe(
      String(RAIL_WIDTH_DEFAULT + RAIL_WIDTH_STEP),
    );
  });
});

describe("run report rail toggle (#694)", () => {
  it("hides the rail so the detail pane takes the full width, and shows it again", async () => {
    stub();
    const { container } = renderResults();
    await findSplitter();
    const grid = container.querySelector(".run-results-grid") as HTMLElement;
    const rail = container.querySelector("#run-results-rail") as HTMLElement;
    const toggle = screen.getByRole("button", { name: "Hide criteria" });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(grid.className).toContain("md:grid-cols-");

    fireEvent.click(toggle);
    expect(rail.hidden).toBe(true);
    expect(screen.queryByRole("separator", { name: "Resize the step list" })).toBeNull();
    expect(grid.className).not.toContain("md:grid-cols-");
    expect(grid.getAttribute("data-rail")).toBe("hidden");
    // Hidden, not unmounted: the same rail comes back with its state.
    expect(container.querySelector("#run-results-rail")).toBe(rail);

    fireEvent.click(screen.getByRole("button", { name: "Show criteria" }));
    expect(rail.hidden).toBe(false);
    expect(await findSplitter()).toBeTruthy();
  });

  it("remembers a hidden rail across a remount", async () => {
    stub();
    const first = renderResults();
    await findSplitter();
    fireEvent.click(screen.getByRole("button", { name: "Hide criteria" }));
    expect(localStorage.getItem(HIDDEN_KEY)).toBe("true");
    first.unmount();

    stub();
    const { container } = renderResults();
    expect(await screen.findByRole("button", { name: "Show criteria" })).toBeTruthy();
    expect((container.querySelector("#run-results-rail") as HTMLElement).hidden).toBe(true);
  });
});
