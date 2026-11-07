import { cn } from "@open-lakehouse/ui-kit";
import { Sparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  capabilityIndex,
  capabilityLabel,
  conflictsWith,
  groupByCategory,
  isGatewayManaged,
  slotHolders,
} from "../selection/catalogIndex.js";
import { describeOrigin, formatList, type Origin } from "../state/origin.js";
import type { CatalogDto, ModuleDto, Selection } from "../types.js";
import {
  PaletteDialog,
  PaletteGroup,
  PaletteRow,
  usePaletteNav,
} from "./paletteList.js";
import { ModuleIcon } from "./projectIcons.js";
import { roleStyle } from "./roleTheme.js";

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
  /** Ask for a role (capability) and let the planner pick its provider, or
   *  drop one asked for. */
  onToggleCapability: (capability: string) => void;
}

/** What picking a palette row does, or why it can't be picked. */
export type RowAction =
  | { kind: "add" }
  | { kind: "remove" }
  | { kind: "replace"; fromId: string; fromName: string }
  /** Take over a one-per-environment slot (e.g. the object store). */
  | { kind: "switch"; fromId: string; fromName: string }
  | { kind: "unavailable"; reason: string };

export interface ModuleRow {
  type: "module";
  key: string;
  module: ModuleDto;
  action: RowAction;
}

/** "Any provider of this role": the planner picks. */
export interface CapabilityRow {
  type: "capability";
  key: string;
  capability: string;
  providers: ModuleDto[];
  action: RowAction;
}

export type Row = ModuleRow | CapabilityRow;

/** The edit picking a row asks for. */
export type RowIntent =
  | { kind: "add"; moduleId: string }
  | { kind: "remove"; moduleId: string }
  | { kind: "swap"; fromId: string; toId: string }
  | { kind: "toggleCapability"; capability: string };

/** What picking `row` does, or `undefined` when it can't be picked. */
export function rowIntent(row: Row): RowIntent | undefined {
  const { action } = row;
  if (action.kind === "unavailable") return undefined;
  if (row.type === "capability")
    return { kind: "toggleCapability", capability: row.capability };
  if (action.kind === "replace" || action.kind === "switch")
    return { kind: "swap", fromId: action.fromId, toId: row.module.id };
  if (action.kind === "remove")
    return { kind: "remove", moduleId: row.module.id };
  return { kind: "add", moduleId: row.module.id };
}

/**
 * Add module…: a searchable picker over the catalog. Add a module, replace a
 * conflicting one, switch the provider of a one-per-environment slot (the
 * database, the object store), or remove one you added. The role chips (or a
 * `role:<name>` query) narrow it to one role's providers and offer "any
 * provider", which leaves the pick to the planner. Browsing lists what's in
 * the environment first; a query starting with "remove" narrows to what can
 * be removed. Arrow keys move, Enter picks.
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
  const [role, setRole] = useState<string | undefined>();
  useEffect(() => {
    if (open) {
      setQuery("");
      setRole(undefined);
    }
  }, [open]);

  const roles = useMemo(
    () => (catalog ? capabilityIndex(catalog).map(([r]) => r) : []),
    [catalog],
  );
  const groups = useMemo(
    () =>
      catalog
        ? paletteRows(catalog, selection, planned, originOf, query, role)
        : { capability: undefined, modules: [] },
    [catalog, selection, planned, originOf, query, role],
  );
  const pickable = useMemo(
    () =>
      [
        ...(groups.capability ? [groups.capability] : []),
        ...groups.modules.flatMap(([, rows]) => rows),
      ].filter((r) => r.action.kind !== "unavailable"),
    [groups],
  );

  const pick = (row: Row) => {
    const intent = rowIntent(row);
    if (!intent) return;
    if (intent.kind === "toggleCapability")
      onToggleCapability(intent.capability);
    else if (intent.kind === "swap") onSwap(intent.fromId, intent.toId);
    else if (intent.kind === "remove") onRemove(intent.moduleId);
    else onAdd(intent.moduleId);
    // Stay open after a removal, so several can go in a row.
    if (intent.kind !== "remove") onOpenChange(false);
  };

  // Enter on a fresh palette adds, never removes: the default highlight skips
  // removals unless the query asks for them.
  const nav = usePaletteNav({
    open,
    items: pickable,
    keyOf: (r) => r.key,
    fallback: Math.max(
      0,
      pickable.findIndex((r) => r.action.kind !== "remove"),
    ),
    onPick: pick,
  });

  const empty = !groups.capability && groups.modules.length === 0;
  const rowProps = (row: Row) => ({
    itemKey: row.key,
    active: row.key === nav.activeKey,
    disabled: row.action.kind === "unavailable",
    onPick: () => pick(row),
    onHover: () => nav.hover(row),
    ...actionLabel(row.action, row.key === nav.activeKey),
  });

  return (
    <PaletteDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Add module…"
      description="Search the catalog to add a module to the environment, pick one by role, or remove one you added."
      query={query}
      onQueryChange={(q) => {
        setQuery(q);
        nav.reset();
      }}
      onKeyDown={nav.onKeyDown}
      placeholder="Add or remove a module, or filter by role…"
      listRef={nav.listRef}
      above={
        roles.length > 0 && (
          <RoleFilter
            roles={roles}
            role={role}
            onRole={(r) => {
              setRole(r);
              nav.reset();
            }}
          />
        )
      }
      hints={[
        "↑↓ to move",
        "↵ to pick",
        "role:… to filter",
        "“remove …” to remove",
        "esc to close",
      ]}
    >
      {empty && (
        <p className="px-3 py-6 text-center text-sm text-muted-foreground">
          Nothing matches “{query}”.
        </p>
      )}
      {groups.capability && (
        <PaletteGroup
          title={`by role: ${capabilityLabel(groups.capability.capability)}`}
        >
          <PaletteRow
            {...rowProps(groups.capability)}
            icon={<Sparkles className="h-4 w-4 text-primary" />}
            title={`Any ${capabilityLabel(groups.capability.capability)} provider`}
            subtitle={`Let the planner pick: ${formatList(groups.capability.providers.map((p) => p.display_name ?? p.id))}`}
          />
        </PaletteGroup>
      )}
      {groups.modules.map(([category, rows]) => (
        <PaletteGroup key={category} title={category.replace(/_/g, " ")}>
          {rows.map((row) => (
            <PaletteRow
              key={row.key}
              {...rowProps(row)}
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
    </PaletteDialog>
  );
}

/** The by-role chips: all modules, or one role's providers. */
function RoleFilter({
  roles,
  role,
  onRole,
}: {
  roles: readonly string[];
  role: string | undefined;
  onRole: (role: string | undefined) => void;
}) {
  const chip = (value: string | undefined, label: string) => (
    <button
      key={value ?? ""}
      type="button"
      aria-pressed={role === value}
      onClick={() => onRole(value)}
      className={cn(
        "shrink-0 rounded-full border px-2 py-0.5 text-[11px] transition-colors",
        role === value
          ? "border-primary/40 bg-primary/10 text-primary"
          : "border-border text-muted-foreground hover:text-foreground",
      )}
    >
      {label}
    </button>
  );
  return (
    <fieldset
      aria-label="Filter by role"
      className="flex gap-1.5 overflow-x-auto border-b border-border px-3 py-2"
    >
      {chip(undefined, "All")}
      {roles.map((r) => chip(r, capabilityLabel(r)))}
    </fieldset>
  );
}

function actionLabel(action: RowAction, active: boolean) {
  return {
    trailing:
      action.kind === "add"
        ? active
          ? "Add ↵"
          : ""
        : action.kind === "remove"
          ? active
            ? "Remove ↵"
            : "Remove"
          : action.kind === "replace"
            ? `Replace ${action.fromName}`
            : action.kind === "switch"
              ? `Switch from ${action.fromName}${active ? " ↵" : ""}`
              : action.reason,
    trailingClassName:
      action.kind === "replace" || action.kind === "switch"
        ? "text-primary"
        : action.kind === "remove"
          ? "text-destructive"
          : undefined,
  };
}

/** The role a `role:<name>` query asks for (by id or label prefix), and the
 *  rest of the query. */
export function parseRoleQuery(
  catalog: CatalogDto,
  query: string,
): { role: string | undefined; rest: string } {
  const m = /^\s*role:(\S*)\s*(.*)$/i.exec(query);
  if (!m) return { role: undefined, rest: query };
  const name = (m[1] ?? "").toLowerCase();
  const role = capabilityIndex(catalog)
    .map(([r]) => r)
    .find(
      (r) =>
        name !== "" &&
        (r.startsWith(name) ||
          capabilityLabel(r).replace(/\s+/g, "_").startsWith(name)),
    );
  return { role, rest: m[2] ?? "" };
}

/**
 * The palette's rows for a query: with a role (`role`, else a `role:` query),
 * its "any provider" row and its providers; otherwise every module, by
 * category.
 */
export function paletteRows(
  catalog: CatalogDto,
  selection: Selection,
  planned: Set<string>,
  originOf: (id: string) => Origin,
  query: string,
  role?: string,
): { capability: CapabilityRow | undefined; modules: [string, ModuleRow[]][] } {
  const asked = parseRoleQuery(catalog, query);
  const byRole = role ?? asked.role;
  // "remove <name>" (or "rm <name>") narrows to what can be removed.
  const raw = asked.rest.trim().toLowerCase();
  const removal = /^(remove|rm)(\s+|$)/.exec(raw);
  const q = removal ? raw.slice(removal[0].length) : raw;
  const matches = (...fields: (string | null | undefined)[]) =>
    q === "" || fields.some((f) => f?.toLowerCase().includes(q));
  const present = new Set([...planned, ...selection.modules]);
  const conflicts = conflictsWith(catalog, present);
  const slots = slotHolders(catalog, present);
  const name = (id: string) =>
    catalog.modules.find((m) => m.id === id)?.display_name ?? id;

  const moduleAction = (m: ModuleDto): RowAction => {
    if (present.has(m.id)) {
      const origin = originOf(m.id);
      return origin.kind === "explicit"
        ? { kind: "remove" }
        : { kind: "unavailable", reason: describeOrigin(origin) };
    }
    // The slot's provider is switched, wherever it came from: an auto-added
    // provider (e.g. the object store Unity Catalog pulled in) is just
    // replaced by the one picked.
    const holder = slots.get(m.id);
    if (holder)
      return { kind: "switch", fromId: holder, fromName: name(holder) };
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
    .filter((m) => !byRole || m.provider_of === byRole)
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

  const providers = byRole
    ? capabilityIndex(catalog).find(([r]) => r === byRole)?.[1]
    : undefined;
  let capability: CapabilityRow | undefined;
  if (byRole && providers) {
    const provided = providers.find((p) => present.has(p.id));
    const action: RowAction = selection.capabilities.includes(byRole)
      ? { kind: "remove" }
      : provided
        ? {
            kind: "unavailable",
            reason: `Via ${provided.display_name ?? provided.id}`,
          }
        : { kind: "add" };
    const row: CapabilityRow = {
      type: "capability",
      key: `c:${byRole}`,
      capability: byRole,
      providers,
      action,
    };
    if (keep(row)) capability = row;
  }

  return { capability, modules };
}

/** 0 when the name or id starts with the query, 1 when it contains it, 2 for
 *  a match elsewhere (summary, category, role). */
function matchRank(m: ModuleDto, q: string): number {
  const names = [m.display_name ?? "", m.id].map((n) => n.toLowerCase());
  if (names.some((n) => n.startsWith(q))) return 0;
  if (names.some((n) => n.includes(q))) return 1;
  return 2;
}
