// The run report rail's draggable width (#436): a per-viewer convenience
// persisted to localStorage, clamped between the pre-#436 default (the
// readable minimum) and ~60% of the viewport so the detail pane stays
// usable. Mirrors `use-width-mode.ts`'s try/catch persistence. Whether the
// rail is hidden (#694) is remembered the same way, under its own key.

import { useCallback, useEffect, useState } from "react";

// 17rem at the app's 16px root — the rail width before #436 introduced
// resizing, kept as both the floor and the double-click reset target.
export const RAIL_WIDTH_DEFAULT = 272;
export const RAIL_WIDTH_STEP = 16;
const RAIL_WIDTH_MAX_RATIO = 0.6;
const STORAGE_KEY = "duhem-rail-width";
const HIDDEN_KEY = "duhem-rail-hidden";

function computeMax(): number {
  const viewport =
    typeof window === "undefined" ? RAIL_WIDTH_DEFAULT * 3 : window.innerWidth;
  return Math.max(RAIL_WIDTH_DEFAULT, Math.round(viewport * RAIL_WIDTH_MAX_RATIO));
}

function clamp(width: number, max: number): number {
  return Math.min(Math.max(width, RAIL_WIDTH_DEFAULT), max);
}

function readStored(max: number): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return RAIL_WIDTH_DEFAULT;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? clamp(parsed, max) : RAIL_WIDTH_DEFAULT;
  } catch {
    // private mode / storage disabled — in-memory default only
    return RAIL_WIDTH_DEFAULT;
  }
}

function persist(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* private mode / storage disabled — in-memory only */
  }
}

function readHidden(): boolean {
  try {
    return localStorage.getItem(HIDDEN_KEY) === "true";
  } catch {
    return false;
  }
}

export function useRailWidth() {
  const [max, setMax] = useState(computeMax);
  const [width, setWidth] = useState(() => readStored(computeMax()));
  const [hidden, setHiddenState] = useState(readHidden);

  useEffect(() => {
    const onResize = () => setMax(computeMax());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // If the viewport shrinks under a wider stored/dragged width, pull the
  // live value back inside the new bound (storage keeps the wider intent).
  useEffect(() => {
    setWidth((current) => clamp(current, max));
  }, [max]);

  const setRailWidth = useCallback(
    (next: number) => {
      setWidth((current) => {
        const clamped = clamp(next, max);
        if (clamped === current) return current;
        persist(STORAGE_KEY, String(clamped));
        return clamped;
      });
    },
    [max],
  );

  const resetRailWidth = useCallback(
    () => setRailWidth(RAIL_WIDTH_DEFAULT),
    [setRailWidth],
  );

  const setHidden = useCallback((next: boolean) => {
    setHiddenState(next);
    persist(HIDDEN_KEY, String(next));
  }, []);

  return {
    width,
    hidden,
    setHidden,
    min: RAIL_WIDTH_DEFAULT,
    max,
    setRailWidth,
    resetRailWidth,
  };
}
