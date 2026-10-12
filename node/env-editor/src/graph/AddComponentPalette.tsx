import {
  cn,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
} from "@open-lakehouse/ui-kit";
import { Search, Sparkles } from "lucide-react";
import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  capabilityIndex,
  capabilityLabel,
  conflictsWith,
  groupByCategory,
} from "../selection/catalogIndex";
import { formatList, type Origin } from "../state/origin";
import type { CatalogDto, ModuleDto, Selection } from "../types";
import { ModuleIcon } from "./projectIcons";
import { roleStyle } from "./roleTheme";

export interface AddComponentPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  catalog: CatalogDto | undefined;
  selection: Selection;
  /** Ids of the components in the current plan. */
  planned: Set<string>;
  originOf: (id: string) => Origin;
  onAdd: (moduleId: string) => void;
  onSwap: (fromId: string, toId: string) => void;
  onAddCapability: (capability: string) => void;
}

/** What picking a palette row does, or why it can't be picked. */
type RowAction =
  | { kind: "add" }
  | { kind: "replace"; fromId: string; fromName: string }
  | { kind: "unavailable"; reason: string };

interface ModuleRow {
  type: "module";
  key: string;
  module: ModuleDto;
  action: RowAction;
}

interface CapabilityRow {
  type: "capability";
  key: string;
  capability: string;
  providers: ModuleDto[];
  action: RowAction;
}

type Row = ModuleRow | CapabilityRow;

/**
 * A searchable command palette over the catalog: add a module, replace a
 * conflicting one, or ask for a capability and let the planner pick its
 * provider. Arrow keys move, Enter picks.
 */
export function AddComponentPalette({
  open,
  onOpenChange,
  catalog,
  selection,
  planned,
  originOf,
  onAdd,
  onSwap,
  onAddCapability,
}: AddComponentPaletteProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
    }
  }, [open]);

  const groups = useMemo(
    () =>
      catalog
        ? paletteRows(catalog, selection, planned, originOf, query)
        : { modules: [], capabilities: [] },
    [catalog, selection, planned, originOf, query],
  );
  const pickable = useMemo(
    () =>
      [
        ...groups.modules.flatMap(([, rows]) => rows),
        ...groups.capabilities,
      ].filter((r) => r.action.kind !== "unavailable"),
    [groups],
  );
  const activeKey = pickable[Math.min(active, pickable.length - 1)]?.key;

  const pick = (row: Row) => {
    if (row.action.kind === "unavailable") return;
    if (row.type === "capability") onAddCapability(row.capability);
    else if (row.action.kind === "replace")
      onSwap(row.action.fromId, row.module.id);
    else onAdd(row.module.id);
    onOpenChange(false);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((i) =>
        pickable.length === 0
          ? 0
          : (Math.min(i, pickable.length - 1) + step + pickable.length) %
            pickable.length,
      );
    } else if (e.key === "Enter") {
      const row = pickable.find((r) => r.key === activeKey);
      if (row) {
        e.preventDefault();
        pick(row);
      }
    }
  };

  const hover = (row: Row) => {
    const i = pickable.indexOf(row);
    if (i >= 0) setActive(i);
  };

  const empty = groups.modules.length === 0 && groups.capabilities.length === 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="top-[15%] max-w-lg translate-y-0 gap-0 overflow-hidden">
        <DialogTitle className="sr-only">Add a component</DialogTitle>
        <DialogDescription className="sr-only">
          Search the catalog and add a module or capability to the environment.
        </DialogDescription>
        <div className="flex items-center gap-2 border-b border-border px-3">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Input
            autoFocus
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
            placeholder="Add a component or capability…"
            className="h-11 border-0 bg-transparent px-0 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
          />
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-1">
          {empty && (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              Nothing in the catalog matches “{query}”.
            </p>
          )}
          {groups.modules.map(([category, rows]) => (
            <PaletteGroup key={category} title={category.replace(/_/g, " ")}>
              {rows.map((row) => (
                <PaletteRow
                  key={row.key}
                  row={row}
                  active={row.key === activeKey}
                  onPick={() => pick(row)}
                  onHover={() => hover(row)}
                  icon={
                    <ModuleIcon
                      moduleId={row.module.id}
                      role={row.module.provider_of}
                      className={cn(
                        "h-4 w-4",
                        roleStyle(row.module.provider_of).accent,
                      )}
                    />
                  }
                  title={row.module.display_name ?? row.module.id}
                  subtitle={row.module.summary ?? undefined}
                />
              ))}
            </PaletteGroup>
          ))}
          {groups.capabilities.length > 0 && (
            <PaletteGroup title="Capabilities">
              {groups.capabilities.map((row) => (
                <PaletteRow
                  key={row.key}
                  row={row}
                  active={row.key === activeKey}
                  onPick={() => pick(row)}
                  onHover={() => hover(row)}
                  icon={<Sparkles className="h-4 w-4 text-primary" />}
                  title={capabilityLabel(row.capability)}
                  subtitle={`Provided by ${formatList(row.providers.map((p) => p.display_name ?? p.id))}`}
                />
              ))}
            </PaletteGroup>
          )}
        </div>
        <div className="flex items-center gap-3 border-t border-border px-3 py-2 text-[11px] text-muted-foreground">
          <span>↑↓ to move</span>
          <span>↵ to add</span>
          <span>esc to close</span>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function PaletteGroup({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="py-1">
      <div className="px-2 pb-1 pt-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      {children}
    </div>
  );
}

function PaletteRow({
  row,
  active,
  onPick,
  onHover,
  icon,
  title,
  subtitle,
}: {
  row: Row;
  active: boolean;
  onPick: () => void;
  onHover: () => void;
  icon: ReactNode;
  title: string;
  subtitle?: string;
}) {
  const { action } = row;
  const unavailable = action.kind === "unavailable";
  return (
    <button
      type="button"
      disabled={unavailable}
      onClick={onPick}
      onMouseMove={onHover}
      className={cn(
        "flex w-full items-center gap-3 rounded-md px-2 py-2 text-left",
        active && "bg-muted",
        unavailable && "cursor-default opacity-55",
      )}
    >
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-muted">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-card-foreground">
          {title}
        </div>
        {subtitle && (
          <div className="truncate text-xs text-muted-foreground">
            {subtitle}
          </div>
        )}
      </div>
      <span
        className={cn(
          "shrink-0 text-[11px]",
          action.kind === "replace" ? "text-primary" : "text-muted-foreground",
        )}
      >
        {action.kind === "add" && (active ? "Add ↵" : "")}
        {action.kind === "replace" && `Replace ${action.fromName}`}
        {action.kind === "unavailable" && action.reason}
      </span>
    </button>
  );
}

/** Build the palette's rows for a query: modules by category, then capabilities. */
function paletteRows(
  catalog: CatalogDto,
  selection: Selection,
  planned: Set<string>,
  originOf: (id: string) => Origin,
  query: string,
): { modules: [string, ModuleRow[]][]; capabilities: CapabilityRow[] } {
  const q = query.trim().toLowerCase();
  const matches = (...fields: (string | null | undefined)[]) =>
    q === "" || fields.some((f) => f?.toLowerCase().includes(q));
  const present = new Set([...planned, ...selection.modules]);
  const conflicts = conflictsWith(catalog, present);
  const name = (id: string) =>
    catalog.modules.find((m) => m.id === id)?.display_name ?? id;

  const moduleAction = (m: ModuleDto): RowAction => {
    if (present.has(m.id)) {
      const origin = originOf(m.id);
      return {
        kind: "unavailable",
        reason: origin.kind === "explicit" ? "Added" : "Included",
      };
    }
    const blockers = conflicts.get(m.id) ?? [];
    if (blockers.length === 0) return { kind: "add" };
    // One conflicting pick the user made can be swapped out in place.
    const [only] = blockers;
    if (blockers.length === 1 && only && originOf(only).kind === "explicit") {
      return { kind: "replace", fromId: only, fromName: name(only) };
    }
    return {
      kind: "unavailable",
      reason: `Conflicts with ${formatList(blockers.map(name))}`,
    };
  };

  const toRow = (module: ModuleDto): ModuleRow => ({
    type: "module",
    key: `m:${module.id}`,
    module,
    action: moduleAction(module),
  });
  const found = catalog.modules.filter((m) =>
    matches(m.display_name, m.id, m.summary, m.category, m.provider_of),
  );
  // Browsing groups by category; a search ranks name matches first in one list.
  const modules: [string, ModuleRow[]][] =
    q === ""
      ? groupByCategory(found).map(([category, list]) => [
          category,
          list.map(toRow),
        ])
      : found.length > 0
        ? [
            [
              "results",
              found
                .map((m) => ({ m, rank: matchRank(m, q) }))
                .sort((a, b) => a.rank - b.rank)
                .map(({ m }) => toRow(m)),
            ],
          ]
        : [];

  const capabilities = capabilityIndex(catalog)
    .filter(([capability, providers]) =>
      matches(
        capability,
        capabilityLabel(capability),
        ...providers.map((p) => p.display_name),
      ),
    )
    .map(([capability, providers]): CapabilityRow => {
      const provided = providers.find((p) => present.has(p.id));
      return {
        type: "capability",
        key: `c:${capability}`,
        capability,
        providers,
        action:
          provided || selection.capabilities.includes(capability)
            ? {
                kind: "unavailable",
                reason: provided
                  ? `Via ${provided.display_name ?? provided.id}`
                  : "Added",
              }
            : { kind: "add" },
      };
    });

  return { modules, capabilities };
}

/** 0 when the name or id starts with the query, 1 when it contains it, 2 for
 *  a match elsewhere (summary, category, role). */
function matchRank(m: ModuleDto, q: string): number {
  const names = [m.display_name ?? "", m.id].map((n) => n.toLowerCase());
  if (names.some((n) => n.startsWith(q))) return 0;
  if (names.some((n) => n.includes(q))) return 1;
  return 2;
}
