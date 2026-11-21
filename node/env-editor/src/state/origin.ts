// Why a component is in the environment: the user picked it, or the planner
// pulled it in for something else. Only an explicit pick can be removed — an
// automatic one leaves when whatever needs it does.

import {
  capabilityLabel,
  isPlatformManaged,
} from "../selection/catalogIndex.js";
import type { CatalogDto, GraphDto, Selection } from "../types.js";

export type Origin =
  | { kind: "explicit" }
  /** Pulled in by the planner; `requiredBy` names what needs it. */
  | { kind: "auto"; requiredBy: string[] }
  /** The gateway itself, or the provider of a platform feature. */
  | { kind: "gateway"; role: string };

/** Derive a planned component's origin from the selection and the plan graph. */
export function componentOrigin(
  id: string,
  selection: Selection,
  catalog: CatalogDto | undefined,
  graph: GraphDto | undefined,
): Origin {
  if (selection.modules.includes(id)) return { kind: "explicit" };
  const module = catalog?.modules.find((m) => m.id === id);
  if (module?.provider_of && isPlatformManaged(module)) {
    return { kind: "gateway", role: module.provider_of };
  }
  const nodes = new Map(graph?.nodes.map((n) => [n.id, n]) ?? []);
  const name = (nodeId: string) => nodes.get(nodeId)?.display_name ?? nodeId;
  const requiredBy = new Set<string>();
  // Hard module dependencies of components in the plan.
  for (const m of catalog?.modules ?? []) {
    if (m.requires.includes(id) && nodes.has(m.id)) requiredBy.add(name(m.id));
  }
  // Components that call it for a resource, sign-in, policy or a platform
  // feature.
  for (const e of graph?.edges ?? []) {
    if (e.to !== id) continue;
    if (
      e.kind === "uses" ||
      e.kind === "authenticates" ||
      e.kind === "authorizes" ||
      e.kind === "integrates"
    ) {
      requiredBy.add(name(e.from));
    }
  }
  // A selected capability it provides.
  const provides = catalog?.modules.find((m) => m.id === id)?.provider_of;
  if (provides && selection.capabilities.includes(provides)) {
    requiredBy.add(`the ${capabilityLabel(provides)} capability`);
  }
  // Extra resources the user asked it to provision.
  const kinds = catalog?.modules.find((m) => m.id === id)?.resource_kinds ?? [];
  const extras = (selection.extra_resources ?? []).filter(
    (e) => e.provider === id || (!e.provider && kinds.includes(e.resource)),
  );
  if (extras.length > 0) {
    requiredBy.add(
      extras.length === 1
        ? `your extra resource ${extras[0]?.name}`
        : `your ${extras.length} extra resources`,
    );
  }
  requiredBy.delete(name(id));
  return { kind: "auto", requiredBy: [...requiredBy] };
}

/** A one-line explanation of an origin, for tooltips and the inspector. */
export function describeOrigin(origin: Origin): string {
  if (origin.kind === "explicit") return "Added by you";
  if (origin.kind === "gateway") {
    if (origin.role === "gateway") return "Every environment has the gateway";
    const feature =
      origin.role === "policy"
        ? "policy"
        : origin.role === "tracing"
          ? "observability"
          : "identity";
    return `Provides the ${feature} platform feature`;
  }
  return origin.requiredBy.length > 0
    ? `Pulled in by ${formatList(origin.requiredBy)}`
    : "Pulled in automatically";
}

/** "A", "A and B", "A, B and C". */
export function formatList(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}
