import { Link, useLocation } from "react-router-dom";

import { cn } from "@/lib/utils";
import { BrandMark } from "./BrandMark";
import { NAV } from "./nav";

// The navigation sidebar body. Used both as the fixed desktop rail and
// inside the mobile Sheet. `brandHeading` is true only for the desktop
// rail so there is exactly one <h1>Duhem</h1> visible per viewport.
export function AppSidebar({
  brandHeading = false,
  onNavigate,
}: {
  brandHeading?: boolean;
  onNavigate?: () => void;
}) {
  // Active state is derived from the item's own `match` predicate, not a
  // `to`-prefix — so `/run/:id` and `/run/:id/check/...` keep "Runs" lit
  // even though the nav target is `/runs` (plural). See nav.ts.
  const { pathname } = useLocation();
  return (
    <div className="flex h-full w-60 flex-col bg-sidebar text-sidebar-foreground">
      <div className="flex h-14 items-center border-b px-4">
        <BrandMark asHeading={brandHeading} onClick={onNavigate} />
      </div>

      <nav className="flex-1 space-y-1 overflow-y-auto p-3">
        {NAV.map((item) => {
          const active = item.match(pathname);
          return (
            <Link
              key={item.to}
              to={item.to}
              onClick={onNavigate}
              aria-current={active ? "page" : undefined}
              className={cn(
                "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                active
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-foreground",
              )}
            >
              <item.icon className="size-4 shrink-0" />
              {item.label}
            </Link>
          );
        })}
      </nav>

      <div className="border-t px-4 py-3 text-xs text-muted-foreground">
        Runs &amp; evidence · read-only
      </div>
    </div>
  );
}
