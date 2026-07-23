import type { ComponentType, ReactNode } from "react";

import { Skeleton } from "@/components/ui/skeleton";

export function ErrorState({ error }: { error: string }) {
  return (
    <div className="rounded-lg border border-fail/30 bg-fail/5 px-4 py-3 text-sm text-fail">
      {error}
    </div>
  );
}

export function EmptyState({
  title,
  hint,
  icon: Icon,
  action,
}: {
  title: ReactNode;
  hint?: ReactNode;
  icon?: ComponentType<{ className?: string }>;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed py-16 text-center">
      {Icon && <Icon className="mb-3 size-8 text-muted-foreground/50" />}
      <p className="text-sm font-medium">{title}</p>
      {hint && (
        <p className="mt-1 max-w-sm text-sm text-muted-foreground">{hint}</p>
      )}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

// Placeholder card grid shown while the runs list loads.
export function CardsSkeleton({ count = 4 }: { count?: number }) {
  return (
    <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
      {Array.from({ length: count }, (_, i) => (
        <Skeleton key={i} className="h-28 rounded-xl" />
      ))}
    </div>
  );
}
