#!/usr/bin/env bun
// Fail if a packed npm tarball depends on a sibling package unresolved: a
// `workspace:` spec, or the `0.0.0` placeholder version `bun pm pack` takes
// from bun.lock when package.json still says `workspace:*` (see
// stamp-versions.ts). Either one makes the package uninstallable without the
// consumer overriding it.
//
//   bun scripts/check-packed.ts <package.tgz>...
import { $ } from "bun";

const DEP_FIELDS = [
  "dependencies",
  "devDependencies",
  "peerDependencies",
  "optionalDependencies",
] as const;

/** The unresolved sibling specs in a package.json, as `field name@spec`. */
export function unresolvedSiblings(pkg: Record<string, unknown>): string[] {
  const bad: string[] = [];
  for (const field of DEP_FIELDS) {
    const deps = (pkg[field] ?? {}) as Record<string, string>;
    for (const [name, spec] of Object.entries(deps)) {
      if (!name.startsWith("@open-lakehouse/")) continue;
      if (spec.startsWith("workspace:") || spec === "0.0.0") {
        bad.push(`${field} ${name}@${spec}`);
      }
    }
  }
  return bad;
}

if (import.meta.main) {
  const tarballs = process.argv.slice(2);
  if (tarballs.length === 0) throw new Error("usage: check-packed.ts <tgz>...");
  let failed = false;
  for (const tgz of tarballs) {
    const pkg = JSON.parse(
      await $`tar -xzOf ${tgz} package/package.json`.text(),
    );
    const bad = unresolvedSiblings(pkg);
    if (bad.length > 0) {
      failed = true;
      console.error(
        `${tgz}: unresolved sibling dependencies: ${bad.join(", ")}`,
      );
    } else {
      console.log(`${tgz}: ${pkg.name}@${pkg.version} pins its siblings`);
    }
  }
  if (failed) process.exit(1);
}
