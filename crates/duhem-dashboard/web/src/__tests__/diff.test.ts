// Pure helpers for the run diff (#212) + visual diff (#213).

import { describe, expect, it } from "vitest";
import { changedDelta, diffPixels, harDelta } from "../diff";

describe("diffPixels (#213)", () => {
  it("reports zero changed for identical buffers", () => {
    const a = new Uint8ClampedArray([10, 20, 30, 255, 40, 50, 60, 255]);
    const b = new Uint8ClampedArray([10, 20, 30, 255, 40, 50, 60, 255]);
    const d = diffPixels(a, b);
    expect(d.changed).toBe(0);
    expect(d.total).toBe(2);
    expect(d.pct).toBe(0);
  });

  it("counts and tints a pixel changed beyond tolerance", () => {
    const a = new Uint8ClampedArray([0, 0, 0, 255, 0, 0, 0, 255]);
    const b = new Uint8ClampedArray([0, 0, 0, 255, 200, 200, 200, 255]);
    const d = diffPixels(a, b);
    expect(d.changed).toBe(1);
    expect(d.pct).toBe(0.5);
    // Changed pixel (2nd, byte offset 4) tinted --fail translucent.
    expect(d.mask[4]).toBe(248);
    expect(d.mask[7]).toBe(160);
    // Unchanged pixel stays transparent.
    expect(d.mask[3]).toBe(0);
  });

  it("ignores sub-tolerance noise (anti-aliasing shimmer)", () => {
    const a = new Uint8ClampedArray([100, 100, 100, 255]);
    const b = new Uint8ClampedArray([108, 100, 100, 255]); // channel-sum diff 8 < 32
    expect(diffPixels(a, b).changed).toBe(0);
  });

  it("throws on a length mismatch rather than reporting a bogus 0%", () => {
    expect(() => diffPixels(new Uint8ClampedArray(4), new Uint8ClampedArray(8))).toThrow();
  });
});

describe("harDelta (#212 network delta)", () => {
  const entry = (method: string, url: string, status: number) => ({
    request: { method, url },
    response: { status },
  });
  const har = (entries: unknown[]) => ({ log: { entries } });

  it("classifies new / removed / status-changed / unchanged", () => {
    const base = har([entry("GET", "/a", 200), entry("GET", "/b", 200), entry("POST", "/gone", 200)]);
    const cur = har([entry("GET", "/a", 200), entry("GET", "/b", 500), entry("POST", "/new", 201)]);
    const rows = harDelta(base, cur);
    const by = Object.fromEntries(rows.map((r) => [r.url, r]));
    expect(by["/a"].kind).toBe("unchanged");
    expect(by["/b"].kind).toBe("status-changed");
    expect(by["/b"].baseStatus).toBe(200);
    expect(by["/b"].curStatus).toBe(500);
    expect(by["/new"].kind).toBe("new");
    expect(by["/gone"].kind).toBe("removed");

    const changed = changedDelta(rows).map((r) => r.url).sort();
    expect(changed).toEqual(["/b", "/gone", "/new"]);
  });

  it("tolerates a malformed HAR shape", () => {
    expect(harDelta({}, { log: {} })).toEqual([]);
    expect(harDelta(null, undefined)).toEqual([]);
  });

  it("drops entries missing a method or url instead of keying a bogus row", () => {
    const base = har([entry("GET", "/a", 200)]);
    // One well-formed entry + one malformed (no url) + one junk object.
    const cur = har([entry("GET", "/a", 200), { request: { method: "GET" }, response: { status: 500 } }, {}]);
    const rows = harDelta(base, cur);
    expect(rows).toHaveLength(1);
    expect(rows[0].url).toBe("/a");
    expect(rows[0].kind).toBe("unchanged");
  });
});
