import { cleanup, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it } from "vitest";
import type { ShardCoverage } from "../api";
import { Shards } from "../views/Shards";
import { verdictWord } from "../verdict-band";

const coverage: ShardCoverage = {
  total: 2, status: "pending", units: ["criterion A"],
  arrived: [{ run_id: "shard-one", index: 1, total: 2, source: "flag", unit_count: 1, verdict: "pass", duration_ms: 1200 }],
  missing: [{ index: 2, units: ["criterion B"] }],
};
afterEach(cleanup);

describe("distributed shards", () => {
  it("shows provenance, verdict, duration, coverage and a navigable shard", () => {
    render(<MemoryRouter><Shards coverage={coverage} /></MemoryRouter>);
    const section = screen.getByRole("region", { name: "Shards" });
    expect(within(section).getByRole("link", { name: "Shard 1/2" }).getAttribute("href")).toBe("/run/shard-one");
    for (const text of ["pass", "1.2s", "Source: flag", "1 units", "1/2 arrived · 1 units covered"]) {
      expect(within(section).getByText(text)).toBeTruthy();
    }
    expect(within(section).getByText("Shard 2/2 · pending · missing")).toBeTruthy();
    expect(within(section).getByRole("list", { name: "Missing units for shard 2" }).textContent).toBe("criterion B");
    expect(verdictWord(null, "pending").word).toBe("Pending");
  });

  it("names missing work as inconclusive after the grace window", () => {
    render(<MemoryRouter><Shards coverage={{ ...coverage, status: "finished" }} /></MemoryRouter>);
    expect(screen.getByText("Shard 2/2 · inconclusive · missing")).toBeTruthy();
  });

  it("omits missing rows when every shard arrived", () => {
    render(<MemoryRouter><Shards coverage={{ ...coverage, status: "finished", missing: [] }} /></MemoryRouter>);
    expect(screen.queryByText(/missing/)).toBeNull();
  });
});
