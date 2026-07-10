// Pure helpers for the run-to-run diff view (#212) and its screenshot
// visual diff (#213). Kept free of the DOM/canvas so they unit-test
// against synthetic inputs. Nothing here influences a verdict — the
// diff is evidence for a human to read, never a judge input.

import type { ArtifactRef } from "./api";

export function pickArtifact(
  artifacts: ArtifactRef[],
  kind: string,
): ArtifactRef | undefined {
  return artifacts.find((a) => a.kind === kind);
}

// ---- #213: screenshot visual diff (pure pixel comparison) ----------

export interface PixelDiff {
  /** Pixels whose channel-sum difference exceeded the tolerance. */
  changed: number;
  total: number;
  /** `changed / total` in [0, 1]. */
  pct: number;
  /** RGBA overlay: changed pixels tinted, others transparent. */
  mask: Uint8ClampedArray;
}

/**
 * Compare two equal-length RGBA buffers. A per-pixel channel-sum
 * tolerance absorbs anti-aliasing noise so the highlight tracks real
 * change, not JPEG shimmer. Returns changed-pixel count, ratio, and a
 * tint mask for overlaying. Presentation only — never a verdict input.
 */
export function diffPixels(
  a: Uint8ClampedArray,
  b: Uint8ClampedArray,
  tolerance = 32,
): PixelDiff {
  const total = Math.floor(a.length / 4);
  const mask = new Uint8ClampedArray(a.length);
  let changed = 0;
  for (let i = 0; i < a.length; i += 4) {
    const d =
      Math.abs(a[i] - b[i]) +
      Math.abs(a[i + 1] - b[i + 1]) +
      Math.abs(a[i + 2] - b[i + 2]);
    if (d > tolerance) {
      changed++;
      // --fail (248,81,73) at ~63% alpha.
      mask[i] = 248;
      mask[i + 1] = 81;
      mask[i + 2] = 73;
      mask[i + 3] = 160;
    }
  }
  return { changed, total, pct: total ? changed / total : 0, mask };
}

// ---- #212: network (HAR) delta -------------------------------------

export interface HarReq {
  method: string;
  url: string;
  status: number;
}

export type DeltaKind = "new" | "removed" | "status-changed" | "unchanged";

export interface DeltaRow {
  method: string;
  url: string;
  baseStatus: number | null;
  curStatus: number | null;
  kind: DeltaKind;
}

function entriesOf(har: unknown): HarReq[] {
  const log = (har as { log?: { entries?: unknown } } | null | undefined)?.log;
  const es = log?.entries;
  if (!Array.isArray(es)) return [];
  return es.map((e) => {
    const ee = e as {
      request?: { method?: unknown; url?: unknown };
      response?: { status?: unknown };
    };
    return {
      method: typeof ee.request?.method === "string" ? ee.request.method : "",
      url: typeof ee.request?.url === "string" ? ee.request.url : "",
      status: typeof ee.response?.status === "number" ? ee.response.status : 0,
    };
  });
}

/**
 * Diff two HAR logs by `method url`: requests newly present, removed,
 * or with a changed status. Order follows the current run, with
 * removed requests appended.
 */
export function harDelta(baseHar: unknown, curHar: unknown): DeltaRow[] {
  const base = entriesOf(baseHar);
  const cur = entriesOf(curHar);
  const key = (r: HarReq) => `${r.method} ${r.url}`;
  const baseByKey = new Map(base.map((r) => [key(r), r]));
  const rows: DeltaRow[] = [];
  const seen = new Set<string>();
  for (const c of cur) {
    const k = key(c);
    seen.add(k);
    const b = baseByKey.get(k);
    if (!b) {
      rows.push({ method: c.method, url: c.url, baseStatus: null, curStatus: c.status, kind: "new" });
    } else if (b.status !== c.status) {
      rows.push({ method: c.method, url: c.url, baseStatus: b.status, curStatus: c.status, kind: "status-changed" });
    } else {
      rows.push({ method: c.method, url: c.url, baseStatus: b.status, curStatus: c.status, kind: "unchanged" });
    }
  }
  for (const b of base) {
    if (!seen.has(key(b))) {
      rows.push({ method: b.method, url: b.url, baseStatus: b.status, curStatus: null, kind: "removed" });
    }
  }
  return rows;
}

/** The delta rows worth surfacing — everything but `unchanged`. */
export function changedDelta(rows: DeltaRow[]): DeltaRow[] {
  return rows.filter((r) => r.kind !== "unchanged");
}
