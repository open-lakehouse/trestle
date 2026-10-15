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

/**
 * The role `module` holds a unique slot for, if any: a resource role it
 * provisions (a database, an object store). An environment has one provider
 * per such role — RustFS *or* Azurite *or* Amazon S3 — so picking another
 * means switching (mirrors the planner's role exclusivity).
 */
export function uniqueSlot(
  module: Pick<ModuleDto, "provider_of" | "resource_kinds">,
): string | undefined {
  const role = module.provider_of;
  return role && module.resource_kinds.includes(role) ? role : undefined;
}

/**
 * For each module that fills a unique slot (see `uniqueSlot`), the module in
 * `present` already holding it, if another one does.
 */
export function slotHolders(
  catalog: CatalogDto,
  present: Set<string>,
): Map<string, string> {
  const holder = new Map<string, string>();
  for (const m of catalog.modules) {
    const role = uniqueSlot(m);
    if (role && present.has(m.id)) holder.set(role, m.id);
  }
  const out = new Map<string, string>();
  for (const m of catalog.modules) {
    const role = uniqueSlot(m);
    const held = role && holder.get(role);
    if (held && held !== m.id) out.set(m.id, held);
  }
  return out;
}

/**
 * The roles the planner fills on its own (mirrors the Rust
 * `Role::GATEWAY_MANAGED`): the always-on gateway, and the sign-in and policy
 * providers the gateway's access settings turn on. Pickers don't offer them.
 */
export const GATEWAY_MANAGED_ROLES: readonly string[] = [
  "gateway",
  "auth",
  "policy",
];

/** Whether the planner adds `module` itself (see `GATEWAY_MANAGED_ROLES`). */
export function isGatewayManaged(module: Pick<ModuleDto, "provider_of">) {
  return GATEWAY_MANAGED_ROLES.includes(module.provider_of ?? "");
}

/** The modules a user picks: the catalog without the gateway-managed ones. */
export function pickableModules(catalog: CatalogDto): ModuleDto[] {
  return catalog.modules.filter((m) => !isGatewayManaged(m));
}

/** The pickable capability → providing modules index, sorted by capability.
 *  Gateway-managed roles are left out: they are gateway settings. */
export function capabilityIndex(catalog: CatalogDto): [string, ModuleDto[]][] {
  const index = new Map<string, ModuleDto[]>();
  for (const m of pickableModules(catalog)) {
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
