import { NavLink } from "react-router-dom";

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
  return (
    <div className="flex h-full w-60 flex-col bg-sidebar text-sidebar-foreground">
      <div className="flex h-14 items-center border-b px-4">
        <BrandMark asHeading={brandHeading} onClick={onNavigate} />
      </div>

      <nav className="flex-1 space-y-1 overflow-y-auto p-3">
        {NAV.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.end}
            onClick={onNavigate}
            className={({ isActive }) =>
              cn(
                "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-sidebar-foreground",
              )
            }
          >
            <item.icon className="size-4 shrink-0" />
            {item.label}
          </NavLink>
        ))}
      </nav>

      <div className="border-t px-4 py-3 text-xs text-muted-foreground">
        Runs &amp; evidence · read-only
      </div>
    </div>
  );
}
