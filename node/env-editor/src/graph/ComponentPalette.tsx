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
  useRef,
  useState,
} from "react";
import {
  capabilityIndex,
  capabilityLabel,
  conflictsWith,
  groupByCategory,
  isGatewayManaged,
} from "../selection/catalogIndex";
import { describeOrigin, formatList, type Origin } from "../state/origin";
import type { CatalogDto, ModuleDto, Selection } from "../types";
import { ModuleIcon } from "./projectIcons";
import { roleStyle } from "./roleTheme";

export interface ComponentPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  catalog: CatalogDto | undefined;
  selection: Selection;
  /** Ids of the components in the current plan. */
  planned: Set<string>;
  originOf: (id: string) => Origin;
  onAdd: (moduleId: string) => void;
  onRemove: (moduleId: string) => void;
  onSwap: (fromId: string, toId: string) => void;
  /** Add a capability, or remove a selected one. */
  onToggleCapability: (capability: string) => void;
}

/** What picking a palette row does, or why it can't be picked. */
type RowAction =
  | { kind: "add" }
  | { kind: "remove" }
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
 * conflicting one, remove one you added, or ask for a capability and let the
 * planner pick its provider. Browsing lists what's in the environment first;
 * a query starting with "remove" narrows to what can be removed. Arrow keys
 * move, Enter picks.
 */
export function ComponentPalette({
  open,
  onOpenChange,
  catalog,
  selection,
  planned,
  originOf,
  onAdd,
  onRemove,
  onSwap,
  onToggleCapability,
}: ComponentPaletteProps) {
  const [query, setQuery] = useState("");
  /** The highlighted pickable row; `null` follows the default. */
  const [active, setActive] = useState<number | null>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(null);
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
  // Enter on a fresh palette adds, never removes: the default highlight skips
  // removals unless the query asks for them.
  const fallback = Math.max(
    0,
    pickable.findIndex((r) => r.action.kind !== "remove"),
  );
  const current = Math.min(active ?? fallback, pickable.length - 1);
  const activeKey = pickable[current]?.key;

  // Keep the highlighted row in view as the arrow keys move it. Only for the
  // keyboard: scrolling a half-visible row under the pointer would jitter.
  const listRef = useRef<HTMLDivElement>(null);
  const movedByKey = useRef(false);
  useEffect(() => {
    if (!activeKey || !movedByKey.current) return;
    listRef.current
      ?.querySelector(`[data-key="${CSS.escape(activeKey)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeKey]);

  const pick = (row: Row) => {
    const { action } = row;
    if (action.kind === "unavailable") return;
    if (row.type === "capability") onToggleCapability(row.capability);
    else if (action.kind === "replace") onSwap(action.fromId, row.module.id);
    else if (action.kind === "remove") onRemove(row.module.id);
    else onAdd(row.module.id);
    // Stay open after a removal, so several can go in a row.
    if (action.kind !== "remove") onOpenChange(false);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      movedByKey.current = true;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive(
        pickable.length === 0
          ? null
          : (current + step + pickable.length) % pickable.length,
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
    if (i < 0 || i === current) return;
    movedByKey.current = false;
    setActive(i);
  };

  const empty = groups.modules.length === 0 && groups.capabilities.length === 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="top-[15%] max-w-lg translate-y-0 gap-0 overflow-hidden">
        <DialogTitle className="sr-only">Add or remove components</DialogTitle>
        <DialogDescription className="sr-only">
          Search the catalog to add a module or capability to the environment,
          or remove one you added.
        </DialogDescription>
        <div className="flex items-center gap-2 border-b border-border px-3">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Input
            autoFocus
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(null);
            }}
            onKeyDown={onKeyDown}
            placeholder="Add or remove a component or capability…"
            className="h-11 border-0 bg-transparent px-0 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
          />
        </div>
        <div ref={listRef} className="max-h-[60vh] overflow-y-auto p-1">
          {empty && (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              Nothing matches “{query}”.
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
          <span>↵ to pick</span>
          <span>“remove …” to remove</span>
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
      data-key={row.key}
      aria-current={active || undefined}
      disabled={unavailable}
      onClick={onPick}
      onMouseMove={onHover}
      className={cn(
        // The top scroll margin keeps a group's title visible when scrolling
        // up onto its first row.
        "flex w-full scroll-mb-1 scroll-mt-8 items-center gap-3 rounded-md px-2 py-2 text-left transition-colors",
        active
          ? "bg-primary/[0.07] ring-1 ring-inset ring-primary/25"
          : !unavailable && "hover:bg-muted/60",
        unavailable && "cursor-default opacity-55",
      )}
    >
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-md",
          active ? "bg-background" : "bg-muted",
        )}
      >
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
          "max-w-[45%] shrink-0 truncate text-[11px]",
          action.kind === "replace"
            ? "text-primary"
            : action.kind === "remove"
              ? "text-destructive"
              : "text-muted-foreground",
        )}
      >
        {action.kind === "add" && (active ? "Add ↵" : "")}
        {action.kind === "remove" && (active ? "Remove ↵" : "Remove")}
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
  // "remove <name>" (or "rm <name>") narrows to what can be removed.
  const raw = query.trim().toLowerCase();
  const removal = /^(remove|rm)(\s+|$)/.exec(raw);
  const q = removal ? raw.slice(removal[0].length) : raw;
  const matches = (...fields: (string | null | undefined)[]) =>
    q === "" || fields.some((f) => f?.toLowerCase().includes(q));
  const present = new Set([...planned, ...selection.modules]);
  const conflicts = conflictsWith(catalog, present);
  const name = (id: string) =>
    catalog.modules.find((m) => m.id === id)?.display_name ?? id;

  const moduleAction = (m: ModuleDto): RowAction => {
    if (present.has(m.id)) {
      const origin = originOf(m.id);
      return origin.kind === "explicit"
        ? { kind: "remove" }
        : { kind: "unavailable", reason: describeOrigin(origin) };
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
  const keep = (row: { action: RowAction }) =>
    !removal || row.action.kind === "remove";
  // Gateway-managed modules show only once they're in the environment: they
  // come from its access settings, not from picking.
  const found = catalog.modules
    .filter((m) => present.has(m.id) || !isGatewayManaged(m))
    .filter((m) =>
      matches(m.display_name, m.id, m.summary, m.category, m.provider_of),
    )
    .map(toRow)
    .filter(keep);
  // Browsing lists the environment first, then the catalog by category; a
  // search ranks name matches first in one list.
  const modules: [string, ModuleRow[]][] = [];
  if (q === "") {
    const inEnv = found.filter((r) => present.has(r.module.id));
    if (inEnv.length > 0) modules.push(["in this environment", inEnv]);
    const rest = found.filter((r) => !present.has(r.module.id));
    for (const [category, list] of groupByCategory(rest.map((r) => r.module))) {
      modules.push([
        category,
        list.flatMap((m) => rest.filter((r) => r.module === m)),
      ]);
    }
  } else if (found.length > 0) {
    modules.push([
      "results",
      found
        .map((row) => ({ row, rank: matchRank(row.module, q) }))
        .sort((a, b) => a.rank - b.rank)
        .map(({ row }) => row),
    ]);
  }

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
      const action: RowAction = selection.capabilities.includes(capability)
        ? { kind: "remove" }
        : provided
          ? {
              kind: "unavailable",
              reason: `Via ${provided.display_name ?? provided.id}`,
            }
          : { kind: "add" };
      return {
        type: "capability",
        key: `c:${capability}`,
        capability,
        providers,
        action,
      };
    })
    .filter(keep);

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
