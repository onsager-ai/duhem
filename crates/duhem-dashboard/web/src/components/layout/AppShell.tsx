import { useEffect, useState, type ReactNode } from "react";

import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { AppSidebar } from "./AppSidebar";
import { CommandMenu } from "./CommandMenu";
import { TopBar } from "./TopBar";

// The persistent app frame: fixed sidebar (desktop) / drawer (mobile),
// sticky top bar, and a centered content column. Wraps every route.
export function AppShell({ children }: { children: ReactNode }) {
  const [mobileNav, setMobileNav] = useState(false);
  const [cmdkOpen, setCmdkOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.key === "k" || e.key === "K") && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setCmdkOpen((o) => !o);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="min-h-screen">
      <aside className="fixed inset-y-0 left-0 z-40 hidden w-60 border-r md:block">
        <AppSidebar brandHeading />
      </aside>

      <Sheet open={mobileNav} onOpenChange={setMobileNav}>
        <SheetContent side="left" className="w-60 p-0">
          <SheetTitle className="sr-only">Navigation</SheetTitle>
          <AppSidebar onNavigate={() => setMobileNav(false)} />
        </SheetContent>
      </Sheet>

      <div className="flex min-h-screen flex-col md:pl-60">
        <TopBar
          onMenu={() => setMobileNav(true)}
          onSearch={() => setCmdkOpen(true)}
        />
        <main className="mx-auto w-full max-w-6xl flex-1 px-4 py-6 md:px-8 md:py-8">
          {children}
        </main>
      </div>

      <CommandMenu open={cmdkOpen} onOpenChange={setCmdkOpen} />
    </div>
  );
}
