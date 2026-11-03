// What changed between two selections (two revisions, or a revision and the
// draft), as the revision diff view lists it. Pure, so the rules are testable.

import type { Selection } from "../types.js";

/** Ids added to and removed from a list. */
export interface ListDiff {
  added: string[];
  removed: string[];
}

/** One knob whose override changed. `undefined` means no override (the
 *  module's default). */
export interface KnobChange {
  module: string;
  knob: string;
  from: string | undefined;
  to: string | undefined;
}

/** Every difference between two selections. */
export interface SelectionDiff {
  modules: ListDiff;
  capabilities: ListDiff;
  knobs: KnobChange[];
  /** The other settings (`gateway`, `hosted`, `client`, `extra_resources`)
   *  that differ, by field name. */
  settings: string[];
}

/** The settings compared as wholes, besides modules, capabilities and knobs. */
const SETTINGS = ["gateway", "hosted", "client", "extra_resources"] as const;

function listDiff(from: readonly string[], to: readonly string[]): ListDiff {
  const before = new Set(from);
  const after = new Set(to);
  return {
    added: [...after].filter((id) => !before.has(id)).sort(),
    removed: [...before].filter((id) => !after.has(id)).sort(),
  };
}

/** JSON with sorted keys, so key order doesn't count as a change. */
function stable(value: unknown): string {
  return JSON.stringify(value ?? null, (_key, v: unknown) =>
    v && typeof v === "object" && !Array.isArray(v)
      ? Object.fromEntries(
          Object.entries(v as Record<string, unknown>).sort(([a], [b]) =>
            a < b ? -1 : a > b ? 1 : 0,
          ),
        )
      : v,
  );
}

/** What changed from `from` to `to`. */
export function diffSelections(from: Selection, to: Selection): SelectionDiff {
  const knobs: KnobChange[] = [];
  const modules = new Set([
    ...Object.keys(from.knob_overrides),
    ...Object.keys(to.knob_overrides),
  ]);
  for (const module of [...modules].sort()) {
    const a = from.knob_overrides[module] ?? {};
    const b = to.knob_overrides[module] ?? {};
    for (const knob of [
      ...new Set([...Object.keys(a), ...Object.keys(b)]),
    ].sort()) {
      if (a[knob] !== b[knob]) {
        knobs.push({ module, knob, from: a[knob], to: b[knob] });
      }
    }
  }
  return {
    modules: listDiff(from.modules, to.modules),
    capabilities: listDiff(from.capabilities, to.capabilities),
    knobs,
    settings: SETTINGS.filter((f) => stable(from[f]) !== stable(to[f])),
  };
}

/** Whether a diff found nothing. */
export function isEmptyDiff(diff: SelectionDiff): boolean {
  return (
    diff.modules.added.length === 0 &&
    diff.modules.removed.length === 0 &&
    diff.capabilities.added.length === 0 &&
    diff.capabilities.removed.length === 0 &&
    diff.knobs.length === 0 &&
    diff.settings.length === 0
  );
}
