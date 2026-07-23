import { Maximize2, Menu, Minimize2, Search } from "lucide-react";

import { Button } from "@/components/ui/button";
import { BrandMark } from "./BrandMark";
import { Breadcrumbs } from "./Breadcrumbs";
import { LiveIndicator } from "./LiveIndicator";
import { ThemeToggle } from "./ThemeToggle";

export function TopBar({
  onMenu,
  onSearch,
  wide,
  onToggleWidth,
}: {
  onMenu: () => void;
  onSearch: () => void;
  wide: boolean;
  onToggleWidth: () => void;
}) {
  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/80 px-4 backdrop-blur supports-[backdrop-filter]:bg-background/60 md:px-6">
      {/* Mobile: menu button + brand heading (the visible <h1>Duhem</h1>). */}
      <Button
        variant="ghost"
        size="icon"
        className="md:hidden"
        onClick={onMenu}
        aria-label="Open navigation"
      >
        <Menu className="size-5" />
      </Button>
      <div className="md:hidden">
        <BrandMark asHeading />
      </div>

      {/* Desktop: breadcrumbs. */}
      <div className="hidden md:block">
        <Breadcrumbs />
      </div>

      <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={onSearch}
          className="gap-2 text-muted-foreground"
        >
          <Search className="size-4" />
          <span className="hidden sm:inline">Search…</span>
          <kbd className="hidden rounded border bg-muted px-1.5 font-mono text-[10px] leading-5 sm:inline">
            ⌘K
          </kbd>
        </Button>
        <LiveIndicator />
        <Button
          variant="ghost"
          size="icon"
          className="hidden md:inline-flex"
          onClick={onToggleWidth}
          aria-pressed={wide}
          aria-label={wide ? "Use centered width" : "Use full width"}
          title={wide ? "Centered width" : "Full width"}
        >
          {wide ? <Minimize2 className="size-4" /> : <Maximize2 className="size-4" />}
        </Button>
        <ThemeToggle />
      </div>
    </header>
  );
}
