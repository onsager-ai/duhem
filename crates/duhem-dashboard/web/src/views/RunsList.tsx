// Runs list (#86, reskinned #284): faceted filters held in URL state
// (bookmarkable), run-set rows expanding to their leaves (#49), "live"
// badges on in-progress runs (#84) — now on TanStack Table + shadcn.
// Data comes from the shell's shared, visibility-polled runs context
// (#298/#303), so the list stays live without its own fetch.

import { useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  ChevronsUpDown,
  ChevronUp,
  Inbox,
  Layers3,
  List,
  Siren,
  X,
} from "lucide-react";
import { Link, useSearchParams } from "react-router-dom";
import {
  flexRender,
  getCoreRowModel,
  getExpandedRowModel,
  getSortedRowModel,
  useReactTable,
  type Column,
  type ColumnDef,
  type ExpandedState,
  type SortingState,
} from "@tanstack/react-table";

import { PageHeader } from "@/components/layout/PageHeader";
import { EmptyState, ErrorState } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { useRunsData } from "@/runs-context";
import { flatLeaves } from "@/stats";
import {
  formatDuration,
  formatStartedAt,
  verdictFamily,
  VerdictBadge,
} from "@/ui";
import type { RunsListEntry } from "../api";

const VERDICT_CHIPS = ["pass", "fail", "inconclusive", "live"] as const;
const ALL = "__all__";
type RunsView = "runs" | "verification" | "triage";

function runsView(value: string | null): RunsView {
  return value === "verification" || value === "triage" ? value : "runs";
}

function startedMs(entry: RunsListEntry): number {
  return entry.started_at ? Date.parse(entry.started_at) || 0 : 0;
}

export function triageRank(entry: RunsListEntry): number {
  if (entry.live) return 0;
  const family = verdictFamily(entry.verdict);
  if (family === "fail") return 1;
  if (family === "inconclusive") return 2;
  if (family === null) return 2;
  return 3;
}

export function matchesFilters(
  entry: RunsListEntry,
  verification: string,
  verdicts: string[],
  from: string,
  to: string,
): boolean {
  if (verification && entry.verification !== verification) return false;
  if (verdicts.length > 0) {
    const v = entry.verdict;
    const hit =
      (verdicts.includes("live") && entry.live) ||
      (v !== null &&
        verdicts.some(
          (w) => w !== "live" && (v === w || v.startsWith(`${w}:`)),
        ));
    if (!hit) return false;
  }
  if (entry.started_at) {
    const t = entry.started_at.slice(0, 10);
    if (from && t < from) return false;
    if (to && t > to) return false;
  } else if (from || to) {
    return false;
  }
  return true;
}

type Meta = { className?: string };

function SortHeader({
  column,
  children,
}: {
  column: Column<RunsListEntry, unknown>;
  children: React.ReactNode;
}) {
  const sorted = column.getIsSorted();
  return (
    <button
      type="button"
      onClick={() => column.toggleSorting(sorted === "asc")}
      className="inline-flex items-center gap-1 hover:text-foreground"
    >
      {children}
      {sorted === "asc" ? (
        <ChevronUp className="size-3.5" />
      ) : sorted === "desc" ? (
        <ChevronDown className="size-3.5" />
      ) : (
        <ChevronsUpDown className="size-3.5 opacity-50" />
      )}
    </button>
  );
}

const columns: ColumnDef<RunsListEntry>[] = [
  {
    id: "run",
    header: "Run",
    enableSorting: false,
    cell: ({ row }) => {
      const e = row.original;
      const pad = { paddingLeft: `${row.depth * 1.25}rem` };
      if (e.kind === "run-set") {
        return (
          <div className="flex items-center gap-1.5" style={pad}>
            <button
              type="button"
              onClick={row.getToggleExpandedHandler()}
              aria-label={row.getIsExpanded() ? "Collapse" : "Expand"}
              aria-expanded={row.getIsExpanded()}
              className="grid size-5 place-items-center rounded text-muted-foreground hover:bg-muted"
            >
              <ChevronRight
                className={cn(
                  "size-4 transition-transform",
                  row.getIsExpanded() && "rotate-90",
                )}
              />
            </button>
            <span className="font-semibold">{e.verification}</span>
            <span className="text-xs text-muted-foreground">
              {e.children?.length ?? 0} runs
            </span>
          </div>
        );
      }
      return (
        <div style={pad}>
          <Link
            to={`/run/${encodeURIComponent(e.run_id)}`}
            className="block max-w-[22ch] truncate font-mono text-xs hover:underline sm:max-w-[30ch]"
            title={e.run_id}
          >
            {e.run_id}
          </Link>
        </div>
      );
    },
  },
  {
    id: "verification",
    accessorFn: (r) => r.verification,
    header: ({ column }) => <SortHeader column={column}>Verification</SortHeader>,
    cell: ({ row }) => (
      <Link
        to={`/verification/${encodeURIComponent(row.original.verification)}`}
        className="block max-w-[24ch] truncate hover:underline"
        title={row.original.verification}
      >
        {row.original.verification}
      </Link>
    ),
  },
  {
    id: "started",
    accessorFn: (r) => (r.started_at ? Date.parse(r.started_at) || 0 : 0),
    header: ({ column }) => <SortHeader column={column}>Started</SortHeader>,
    cell: ({ row }) => (
      <span className="text-muted-foreground">
        {formatStartedAt(row.original.started_at)}
      </span>
    ),
    meta: { className: "hidden md:table-cell" } satisfies Meta,
  },
  {
    id: "duration",
    accessorFn: (r) => r.duration_ms ?? -1,
    header: ({ column }) => <SortHeader column={column}>Duration</SortHeader>,
    cell: ({ row }) => (
      <span className="text-muted-foreground tabular-nums">
        {formatDuration(row.original.duration_ms)}
      </span>
    ),
    meta: { className: "hidden sm:table-cell" } satisfies Meta,
  },
  {
    id: "verdict",
    header: "Verdict",
    enableSorting: false,
    cell: ({ row }) => (
      <VerdictBadge
        verdict={row.original.verdict}
        live={row.original.live}
        compact
      />
    ),
  },
];

function RunsTable({
  data,
  label,
  initialSorting = [{ id: "started", desc: true }],
}: {
  data: RunsListEntry[];
  label: string;
  initialSorting?: SortingState;
}) {
  const [sorting, setSorting] = useState<SortingState>(initialSorting);
  const [expanded, setExpanded] = useState<ExpandedState>({});

  const table = useReactTable({
    data,
    columns,
    state: { sorting, expanded },
    onSortingChange: setSorting,
    onExpandedChange: setExpanded,
    getSubRows: (row) => (row.kind === "run-set" ? row.children : undefined),
    getRowId: (row, index, parent) =>
      `${parent ? `${parent.id}.` : ""}${row.kind}:${row.run_id}:${index}`,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getExpandedRowModel: getExpandedRowModel(),
  });

  return (
    <div className="min-w-0 border-y" data-testid="runs-table">
      <table className="w-full table-fixed caption-bottom text-sm" aria-label={label}>
        <thead>
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id}>
              {hg.headers.map((h) => (
                <th
                  key={h.id}
                  className={cn(
                    "sticky top-[5.75rem] z-20 h-10 border-b bg-background/95 px-3 text-left align-middle text-xs font-medium text-muted-foreground backdrop-blur",
                    (h.column.columnDef.meta as Meta | undefined)?.className,
                  )}
                >
                  {h.isPlaceholder
                    ? null
                    : flexRender(h.column.columnDef.header, h.getContext())}
                </th>
              ))}
            </tr>
          ))}
        </thead>
        <tbody>
          {table.getRowModel().rows.map((row) => (
            <tr
              key={row.id}
              className={cn(
                "border-b transition-colors last:border-0 hover:bg-muted/40",
                row.depth > 0 && "bg-muted/10",
              )}
            >
              {row.getVisibleCells().map((cell) => (
                <td
                  key={cell.id}
                  className={cn(
                    "px-3 py-2.5 align-middle",
                    (cell.column.columnDef.meta as Meta | undefined)?.className,
                  )}
                >
                  {flexRender(cell.column.columnDef.cell, cell.getContext())}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function TriageView({ leaves }: { leaves: RunsListEntry[] }) {
  const ordered = [...leaves].sort(
    (a, b) => triageRank(a) - triageRank(b) || startedMs(b) - startedMs(a),
  );
  const actionable = ordered.filter((entry) => triageRank(entry) < 3);
  const passing = ordered.filter((entry) => triageRank(entry) === 3);
  return (
    <div className="space-y-8" data-testid="triage-view">
      <section aria-labelledby="needs-attention">
        <div className="mb-2 flex items-baseline justify-between gap-3">
          <h3 id="needs-attention" className="text-sm font-semibold">
            Needs attention
          </h3>
          <span className="text-xs text-muted-foreground">
            live · failed · inconclusive
          </span>
        </div>
        {actionable.length > 0 ? (
          <RunsTable
            data={actionable}
            label="Runs needing attention"
            initialSorting={[]}
          />
        ) : (
          <p className="border-y py-5 text-sm text-muted-foreground">
            No live, failed, or inconclusive runs match the filters.
          </p>
        )}
      </section>
      {passing.length > 0 && (
        <section aria-labelledby="passing-history">
          <div className="mb-2 flex items-baseline justify-between gap-3">
            <h3 id="passing-history" className="text-sm font-semibold">
              Passing history
            </h3>
            <span className="text-xs text-muted-foreground">
              {passing.length} run{passing.length === 1 ? "" : "s"}
            </span>
          </div>
          <RunsTable data={passing} label="Passing run history" />
        </section>
      )}
    </div>
  );
}

export default function RunsList() {
  const { runs, error } = useRunsData();
  const [params, setParams] = useSearchParams();

  const verification = params.get("verification") ?? "";
  const verdicts = params.getAll("verdict");
  const from = params.get("from") ?? "";
  const to = params.get("to") ?? "";
  const view = runsView(params.get("view"));

  const verifications = useMemo(
    () =>
      runs
        ? [...new Set(flatLeaves(runs).map((r) => r.verification))].sort()
        : [],
    [runs],
  );

  const filteredLeaves = useMemo(
    () =>
      flatLeaves(runs ?? []).filter((entry) =>
        matchesFilters(entry, verification, verdicts, from, to),
      ),
    [runs, verification, verdicts, from, to],
  );
  const filteredGroups = useMemo(
    () =>
      (runs ?? []).flatMap((entry) => {
        if (entry.kind !== "run-set") {
          return matchesFilters(entry, verification, verdicts, from, to)
            ? [entry]
            : [];
        }
        const children = (entry.children ?? []).filter((child) =>
          matchesFilters(child, verification, verdicts, from, to),
        );
        return children.length > 0 ? [{ ...entry, children }] : [];
      }),
    [runs, verification, verdicts, from, to],
  );

  const update = (mutate: (p: URLSearchParams) => void) => {
    const next = new URLSearchParams(params);
    mutate(next);
    setParams(next, { replace: true });
  };

  const clearAll = () => {
    const next = new URLSearchParams();
    if (view !== "runs") next.set("view", view);
    setParams(next, { replace: true });
  };
  const hasFilters =
    Boolean(verification) || verdicts.length > 0 || Boolean(from) || Boolean(to);

  if (error) return <ErrorState error={error} />;

  return (
    <div className="space-y-6">
      <PageHeader
        title="Runs"
        description="Review runs chronologically, by verification, or by attention needed."
      />

      <nav
        aria-label="run list views"
        className="sticky top-14 z-20 -mx-4 flex gap-5 border-b bg-background/95 px-4 backdrop-blur md:-mx-8 md:px-8"
      >
        {([
          ["runs", "Runs", List],
          ["verification", "By verification", Layers3],
          ["triage", "Triage", Siren],
        ] as const).map(([id, label, Icon]) => (
          <button
            key={id}
            type="button"
            aria-pressed={view === id}
            onClick={() =>
              update((next) => {
                if (id === "runs") next.delete("view");
                else next.set("view", id);
              })
            }
            className={cn(
              "relative inline-flex h-10 items-center gap-1.5 border-b-2 px-0.5 text-sm font-medium transition-colors",
              view === id
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon className="size-3.5" aria-hidden="true" />
            {label}
          </button>
        ))}
      </nav>

      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={verification || ALL}
          onValueChange={(v) =>
            update((p) =>
              v === ALL ? p.delete("verification") : p.set("verification", v),
            )
          }
        >
          <SelectTrigger
            size="sm"
            className="w-[13rem]"
            aria-label="verification"
          >
            <SelectValue placeholder="All verifications" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>All verifications</SelectItem>
            {verifications.map((v) => (
              <SelectItem key={v} value={v}>
                {v}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <div className="flex items-center gap-1">
          {VERDICT_CHIPS.map((chip) => {
            const on = verdicts.includes(chip);
            return (
              <Button
                key={chip}
                type="button"
                size="sm"
                variant={on ? "default" : "outline"}
                className="capitalize"
                onClick={() =>
                  update((p) => {
                    const current = p.getAll("verdict");
                    p.delete("verdict");
                    const nextChips = current.includes(chip)
                      ? current.filter((c) => c !== chip)
                      : [...current, chip];
                    nextChips.forEach((c) => p.append("verdict", c));
                  })
                }
              >
                {chip}
              </Button>
            );
          })}
        </div>

        <Input
          type="date"
          aria-label="from"
          value={from}
          className="h-8 w-[9.5rem]"
          onChange={(e) =>
            update((p) =>
              e.target.value ? p.set("from", e.target.value) : p.delete("from"),
            )
          }
        />
        <Input
          type="date"
          aria-label="to"
          value={to}
          className="h-8 w-[9.5rem]"
          onChange={(e) =>
            update((p) =>
              e.target.value ? p.set("to", e.target.value) : p.delete("to"),
            )
          }
        />

        {hasFilters && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            onClick={clearAll}
          >
            <X className="size-4" />
            Clear
          </Button>
        )}
      </div>

      {runs === null ? (
        <Skeleton className="h-64 rounded-xl" />
      ) : filteredLeaves.length === 0 ? (
        <EmptyState
          icon={Inbox}
          title={runs.length > 0 ? "No runs match the filters" : "No runs yet"}
          hint={
            runs.length > 0
              ? "Try clearing a filter to widen the results."
              : "Run a verification to populate this list."
          }
          action={
            hasFilters ? (
              <Button variant="outline" size="sm" onClick={clearAll}>
                Clear filters
              </Button>
            ) : undefined
          }
        />
      ) : (
        view === "triage" ? (
          <TriageView leaves={filteredLeaves} />
        ) : (
          <RunsTable
            data={view === "verification" ? filteredGroups : filteredLeaves}
            label={
              view === "verification"
                ? "Runs grouped by verification"
                : "Runs newest first"
            }
          />
        )
      )}
    </div>
  );
}
