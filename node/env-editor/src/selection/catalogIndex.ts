// Catalog lookups shared by the pickers, the component palette and the canvas.

import type { CatalogDto, ModuleDto } from "../types";

/** Group modules by their `category` (uncategorized last), preserving order. */
export function groupByCategory(modules: ModuleDto[]): [string, ModuleDto[]][] {
  const groups = new Map<string, ModuleDto[]>();
  for (const m of modules) {
    const key = m.category ?? "other";
    const list = groups.get(key) ?? [];
    list.push(m);
    groups.set(key, list);
  }
  return [...groups.entries()].sort(([a], [b]) =>
    a === "other" ? 1 : b === "other" ? -1 : a.localeCompare(b),
  );
}

/**
 * For each module, the modules in `present` it can't coexist with: either side
 * may list the other in `conflicts_with`.
 */
export function conflictsWith(
  catalog: CatalogDto,
  present: Set<string>,
): Map<string, string[]> {
  const out = new Map<string, string[]>();
  const note = (id: string, other: string) => {
    if (id === other) return;
    const list = out.get(id) ?? [];
    if (!list.includes(other)) list.push(other);
    out.set(id, list);
  };
  for (const m of catalog.modules) {
    for (const c of m.conflicts_with) {
      if (present.has(m.id)) note(c, m.id);
      if (present.has(c)) note(m.id, c);
    }
  }
  return out;
}

/** The capability → providing modules index, sorted by capability. */
export function capabilityIndex(catalog: CatalogDto): [string, ModuleDto[]][] {
  const index = new Map<string, ModuleDto[]>();
  for (const m of catalog.modules) {
    if (!m.provider_of) continue;
    const providers = index.get(m.provider_of) ?? [];
    providers.push(m);
    index.set(m.provider_of, providers);
  }
  return [...index.entries()].sort(([a], [b]) => a.localeCompare(b));
}

/** A module's display name, falling back to its id. */
export function moduleName(catalog: CatalogDto | undefined, id: string) {
  return catalog?.modules.find((m) => m.id === id)?.display_name ?? id;
}

/** Render a capability id (`experiment_tracking`) as words. */
export function capabilityLabel(capability: string) {
  return capability.replace(/_/g, " ");
}
