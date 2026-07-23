// The one place a timeline `IconName` (#284 follow-up) becomes a glyph.
// `format.ts` stays pure and names *what a row is*; this maps each key
// to a lucide SVG so every icon is a real, theme-aware vector that
// inherits the row's tone via `currentColor` — no more inline emoji.

import {
  Ban,
  Camera,
  Check,
  CircleCheck,
  CircleSlash,
  Clapperboard,
  Clock,
  Dot,
  FileCode,
  Globe,
  Crosshair,
  Paperclip,
  Play,
  X,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";
import type { IconName } from "../format";

const ICONS: Record<IconName, LucideIcon> = {
  action: Play,
  observed: Dot,
  pass: Check,
  fail: X,
  inconclusive: CircleSlash,
  timeout: Clock,
  "verdict-pass": CircleCheck,
  "verdict-fail": Ban,
  screenshot: Camera,
  dom: FileCode,
  network: Globe,
  target: Crosshair,
  video: Clapperboard,
  attachment: Paperclip,
  unknown: Dot,
};

export function EventIcon({ name, className }: { name: IconName; className?: string }) {
  const Icon = ICONS[name] ?? Dot;
  // The `ev-glyph-<name>` hook lets the verdict icons stay pass/fail
  // coloured even though the check-verdict row's tone is "anchor".
  return <Icon className={cn("ev-glyph", `ev-glyph-${name}`, className)} aria-hidden="true" />;
}
