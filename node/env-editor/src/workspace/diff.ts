// What changed between two selections (two revisions, or a revision and the
// draft), as the revision diff view lists it. Pure, so the rules are testable.

import type {
  HostedResource,
  HostedVendor,
  OutputFile,
  Selection,
} from "../types.js";

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

/** One module whose `hosted` entry changed. `undefined` means no entry: a
 *  hosted provider's default, or a function-capable module's local container. */
export interface HostedChange {
  module: string;
  from: HostedResource | undefined;
  to: HostedResource | undefined;
}

/** Every difference between two selections. */
export interface SelectionDiff {
  modules: ListDiff;
  capabilities: ListDiff;
  knobs: KnobChange[];
  /** The modules whose `hosted` entry changed, sorted by module. */
  hosted: HostedChange[];
  /** The other settings (`gateway`, `client`, `extra_resources`) that differ,
   *  by field name. */
  settings: string[];
}

/** The settings compared as wholes, besides modules, capabilities, knobs and
 *  hosted entries. */
const SETTINGS = ["gateway", "client", "extra_resources"] as const;

/** What each vendor is called (mirrors `HostedVendor`'s display name). */
const VENDOR_LABEL: Record<HostedVendor, string> = {
  generic: "Other server",
  neon: "Neon",
  "aws-rds": "Amazon RDS",
  "azure-postgres": "Azure Database for PostgreSQL",
  "aws-s3": "Amazon S3",
  "azure-storage": "Azure Storage",
  "neon-function": "Neon Function",
};

/** What a hosted entry is called: its vendor, or "(default)" for none. */
export function describeHosted(hosted: HostedResource | undefined): string {
  if (!hosted) return "(default)";
  if (hosted.vendor) return VENDOR_LABEL[hosted.vendor] ?? hosted.vendor;
  return hosted.connection.kind === "function" ? "Neon Function" : "custom";
}

/** One hosted change as the diff view lists it. */
export function describeHostedChange({
  module,
  from,
  to,
}: HostedChange): string {
  const fromFn = from?.connection.kind === "function";
  const toFn = to?.connection.kind === "function";
  if (toFn && !fromFn) return `${module} runs as a Neon Function`;
  if (fromFn && !toFn) {
    return to
      ? `${module} runs on ${describeHosted(to)}`
      : `${module} runs in a local container`;
  }
  if (fromFn && toFn) {
    const url = (h: HostedResource | undefined) =>
      (h?.connection.kind === "function" && h.connection.url) || "(default)";
    return `${module} function URL: ${url(from)} → ${url(to)}`;
  }
  const a = describeHosted(from);
  const b = describeHosted(to);
  return a === b
    ? `${module} connection changed`
    : `${module} connection: ${a} → ${b}`;
}

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
  const hostedA = from.hosted ?? {};
  const hostedB = to.hosted ?? {};
  const hosted: HostedChange[] = [
    ...new Set([...Object.keys(hostedA), ...Object.keys(hostedB)]),
  ]
    .sort()
    .filter((m) => stable(hostedA[m]) !== stable(hostedB[m]))
    .map((module) => ({ module, from: hostedA[module], to: hostedB[module] }));
  return {
    modules: listDiff(from.modules, to.modules),
    capabilities: listDiff(from.capabilities, to.capabilities),
    knobs,
    hosted,
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
    diff.hosted.length === 0 &&
    diff.settings.length === 0
  );
}

/** One rendered file that differs between two plans. */
export interface FileChange {
  path: string;
  /** Absent: the file is new. */
  original?: string;
  /** Absent: the file is gone. */
  modified?: string;
}

/** The files that differ from `before` to `after` (added, removed or
 *  changed), by path. */
export function diffFiles(
  before: readonly OutputFile[],
  after: readonly OutputFile[],
): FileChange[] {
  const was = new Map(before.map((f) => [f.path, f.contents]));
  const now = new Map(after.map((f) => [f.path, f.contents]));
  const paths = [...new Set([...was.keys(), ...now.keys()])].sort();
  return paths
    .filter((path) => was.get(path) !== now.get(path))
    .map((path) => ({
      path,
      original: was.get(path),
      modified: now.get(path),
    }));
}
