import { ShieldCheck } from "lucide-react";
import { useNavigate } from "react-router-dom";

import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { useRunsData } from "@/runs-context";
import { computeStats, verificationSummaries } from "@/stats";
import { VerdictBadge } from "@/ui";
import { NAV } from "./nav";

// ⌘K palette — jump to a section, a verification, or a recent run.
export function CommandMenu({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const { runs } = useRunsData();
  const go = (to: string) => {
    onOpenChange(false);
    navigate(to);
  };

  const recent = runs ? computeStats(runs).recent : [];
  const verifications = runs ? verificationSummaries(runs) : [];

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Search"
      description="Jump to a section, verification, or run"
    >
      <CommandInput placeholder="Search runs and verifications…" />
      <CommandList>
        <CommandEmpty>No matches.</CommandEmpty>

        <CommandGroup heading="Go to">
          {NAV.map((item) => (
            <CommandItem
              key={item.to}
              value={`go ${item.label}`}
              onSelect={() => go(item.to)}
            >
              <item.icon />
              {item.label}
            </CommandItem>
          ))}
        </CommandGroup>

        {verifications.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Verifications">
              {verifications.map((v) => (
                <CommandItem
                  key={v.name}
                  value={`verification ${v.name}`}
                  onSelect={() =>
                    go(`/verification/${encodeURIComponent(v.name)}`)
                  }
                >
                  <ShieldCheck />
                  <span className="truncate">{v.name}</span>
                  <span className="ml-auto">
                    <VerdictBadge
                      verdict={v.latest?.verdict ?? null}
                      live={v.live}
                    />
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}

        {recent.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading="Recent runs">
              {recent.map((r) => (
                <CommandItem
                  key={r.run_id}
                  value={`run ${r.run_id} ${r.verification}`}
                  onSelect={() => go(`/run/${encodeURIComponent(r.run_id)}`)}
                >
                  <span className="truncate font-mono text-xs">{r.run_id}</span>
                  <span className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
                    <span className="hidden sm:inline">{r.verification}</span>
                    <VerdictBadge verdict={r.verdict} live={r.live} />
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}
      </CommandList>
    </CommandDialog>
  );
}
