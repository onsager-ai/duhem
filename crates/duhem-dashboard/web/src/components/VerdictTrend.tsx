import { cn } from "@/lib/utils";
import type { Family } from "@/stats";

function toneClass(f: Family): string {
  if (f === "pass") return "bg-pass";
  if (f === "fail") return "bg-fail";
  if (f === "inconclusive") return "bg-inconclusive";
  return "bg-muted-foreground/25";
}

// Compact status strip: one bar per recent run, oldest → newest.
export function VerdictTrend({
  trend,
  className,
}: {
  trend: Family[];
  className?: string;
}) {
  if (trend.length === 0) {
    return <span className="text-xs text-muted-foreground">no runs yet</span>;
  }
  return (
    <div className={cn("flex items-stretch gap-0.5", className)} aria-hidden>
      {trend.map((f, i) => (
        <span
          key={i}
          className={cn("h-5 w-1.5 rounded-[2px]", toneClass(f))}
        />
      ))}
    </div>
  );
}
