// Selections saved before a catalog module was renamed. The planner migrates
// what it plans (`migrate_selection` in crates/stack-topology/src/manifest.rs),
// but only a copy: the editor edits by node id, so it adopts the rename itself
// wherever a stored selection comes in. Mirrors the Rust function; keep the two
// in step.

import type { HostedResource, Selection } from "../types.js";

/** Unity Catalog's id before it was split by implementation (manifest v6). */
const LEGACY_UNITY_CATALOG = "unity-catalog";
/** The open-source server: what `unity-catalog` ran in a container. */
const UNITY_CATALOG_OSS = "unity-catalog-oss";
/** mangrove's server: what `unity-catalog` ran as a Neon Function. */
const UNITY_CATALOG_RS = "unity-catalog-rs";

/**
 * `selection` with renamed module ids rewritten, or `selection` itself when
 * nothing changed (so a caller can tell by identity).
 *
 * `unity-catalog` becomes `unity-catalog-oss` with its knob overrides, unless
 * it ran as a function: then `unity-catalog-rs`, keeping the function entry
 * and dropping the overrides (they were the OSS server's). When the new id is
 * there too, it is selected once, and its own settings win over the legacy
 * ones, which only fill what it leaves unset.
 */
export function migrateSelection(selection: Selection): Selection {
  const legacy = LEGACY_UNITY_CATALOG;
  const selected = selection.modules.includes(legacy);
  const hosted = selection.hosted?.[legacy];
  const overrides = selection.knob_overrides[legacy];
  if (!selected && !hosted && !overrides) return selection;

  const to =
    hosted?.connection.kind === "function"
      ? UNITY_CATALOG_RS
      : UNITY_CATALOG_OSS;
  const modules = [
    ...new Set(selection.modules.map((m) => (m === legacy ? to : m))),
  ];

  const { [legacy]: _hosted, ...otherHosted } = selection.hosted ?? {};
  const nextHosted: Record<string, HostedResource> = otherHosted;
  if (hosted && !nextHosted[to]) nextHosted[to] = hosted;

  const { [legacy]: _knobs, ...knob_overrides } = selection.knob_overrides;
  if (overrides && to === UNITY_CATALOG_OSS) {
    knob_overrides[to] = { ...overrides, ...knob_overrides[to] };
  }

  const next: Selection = { ...selection, modules, knob_overrides };
  if (selection.hosted) next.hosted = nextHosted;
  return next;
}
