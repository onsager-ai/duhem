// Fetches + parses the run's recorded VD snapshot once (#302) and shares
// the resulting lookup with the whole run report via context, so the
// tree rail and the check evidence can overlay authored descriptions +
// step ids without each re-fetching or the scaffold threading props.

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

import { fetchDefinition } from "../api";
import { parseDefinition, type VdLookup } from "../definition";

const DefinitionContext = createContext<VdLookup | null>(null);

/** The parsed VD snapshot for the current run, or `null` when the run
 *  recorded none (older runs) or it's still loading. */
export function useVd(): VdLookup | null {
  return useContext(DefinitionContext);
}

export function DefinitionProvider({
  runId,
  enabled,
  children,
}: {
  runId: string;
  enabled: boolean;
  children: ReactNode;
}) {
  const [vd, setVd] = useState<VdLookup | null>(null);
  useEffect(() => {
    setVd(null);
    if (!enabled) return;
    let live = true;
    fetchDefinition(runId)
      .then((yaml) => live && setVd(parseDefinition(yaml)))
      .catch(() => live && setVd(null));
    return () => {
      live = false;
    };
  }, [runId, enabled]);
  return <DefinitionContext.Provider value={vd}>{children}</DefinitionContext.Provider>;
}
