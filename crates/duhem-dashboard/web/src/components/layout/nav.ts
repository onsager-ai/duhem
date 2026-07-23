import { LayoutDashboard, ListChecks, ShieldCheck } from "lucide-react";

// Primary navigation — shared by the sidebar and the ⌘K palette.
export const NAV = [
  { to: "/", label: "Overview", icon: LayoutDashboard, end: true },
  { to: "/runs", label: "Runs", icon: ListChecks, end: false },
  { to: "/verifications", label: "Verifications", icon: ShieldCheck, end: false },
] as const;
