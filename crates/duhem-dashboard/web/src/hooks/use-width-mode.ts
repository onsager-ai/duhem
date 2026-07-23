// Content-width preference — "normal" (a readable centered column) or
// "wide" (full-bleed, for dense evidence tables / screenshots). Persisted
// to localStorage; a chassis preference, independent of the theme. Kept
// tiny and dependency-free, mirroring `theme.tsx`.

import { useCallback, useState } from "react";

export type WidthMode = "normal" | "wide";

const STORAGE_KEY = "duhem-width";

function readStored(): WidthMode {
  if (typeof localStorage === "undefined") return "normal";
  return localStorage.getItem(STORAGE_KEY) === "wide" ? "wide" : "normal";
}

export function useWidthMode() {
  const [mode, setMode] = useState<WidthMode>(readStored);
  const toggle = useCallback(() => {
    setMode((m) => {
      const next: WidthMode = m === "wide" ? "normal" : "wide";
      try {
        localStorage.setItem(STORAGE_KEY, next);
      } catch {
        /* private mode / storage disabled — in-memory only */
      }
      return next;
    });
  }, []);
  return { mode, wide: mode === "wide", toggle };
}
