import { LayoutDashboard, ListChecks, ShieldCheck } from "lucide-react";

// Primary navigation — shared by the sidebar and the ⌘K palette.
//
// `match` decides which top-level item is highlighted for the current
// path. It is deliberately broader than the `to` prefix: a run detail
// lives at `/run/:id` (singular) and a check at `/run/:id/check/...`,
// but both belong under "Runs" (`/runs`, plural) — so the rail keeps
// Runs active while you drill into a run's evidence. Same for a single
// verification (`/verification/:name`) under "Verifications".
export const NAV = [
  {
    to: "/",
    label: "Overview",
    icon: LayoutDashboard,
    match: (p: string) => p === "/",
  },
  {
    to: "/runs",
    label: "Runs",
    icon: ListChecks,
    match: (p: string) => p === "/runs" || p.startsWith("/run/"),
  },
  {
    to: "/verifications",
    label: "Verifications",
    icon: ShieldCheck,
    match: (p: string) =>
      p === "/verifications" || p.startsWith("/verification/"),
  },
] as const;
