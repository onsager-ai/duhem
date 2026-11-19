import { useSyncExternalStore } from "react";

export interface RunBreadcrumbDetail {
  verification: string;
  run_number: number;
}

const details = new Map<string, RunBreadcrumbDetail>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function notify(): void {
  for (const listener of listeners) listener();
}

export function publishRunBreadcrumbDetail(
  runId: string,
  detail: RunBreadcrumbDetail | null,
): void {
  if (detail === null) {
    if (details.delete(runId)) notify();
    return;
  }

  const previous = details.get(runId);
  if (
    previous?.verification === detail.verification &&
    previous.run_number === detail.run_number
  ) return;
  details.set(runId, detail);
  notify();
}

export function useRunBreadcrumbDetail(runId: string): RunBreadcrumbDetail | null {
  return useSyncExternalStore(
    subscribe,
    () => details.get(runId) ?? null,
    () => null,
  );
}
