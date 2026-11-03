#!/usr/bin/env bun
// Build a publishable workspace package into `dist/`: `tsc -p
// tsconfig.build.json` emits ESM + declarations (+ maps), then the source files
// tsc does not emit (hand-written `.d.ts`, `.json`, `.css`) are copied across
// with their paths kept. Tests and stories are skipped. Finally every bare
// import in `dist/` must be a declared dependency or peer, so a package can't
// ship an import its consumers won't have installed.
//
// Run from the package directory (each package's `build` script does):
//   bun ../scripts/build-package.ts
//
// Workspace consumers never need this: they resolve the `source` export
// condition to `src/` (see tsconfig.base.json and the Vite configs).
import { cpSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { $, Glob } from "bun";

const pkgDir = process.cwd();
const dist = join(pkgDir, "dist");

rmSync(dist, { recursive: true, force: true });
await $`tsc -p tsconfig.build.json`.cwd(pkgDir);

const skip = /\.(test|stories)\.[^/]+$/;
for await (const rel of new Glob("**/*.{d.ts,json,css}").scan({
  cwd: join(pkgDir, "src"),
})) {
  if (skip.test(rel)) continue;
  const to = join(dist, rel);
  mkdirSync(dirname(to), { recursive: true });
  cpSync(join(pkgDir, "src", rel), to);
}

const pkg = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
const declared = new Set<string>([
  pkg.name,
  ...Object.keys(pkg.dependencies ?? {}),
  ...Object.keys(pkg.peerDependencies ?? {}),
]);
/** `@scope/name/sub` → `@scope/name`; `name/sub` → `name`. */
const packageOf = (spec: string) =>
  spec.startsWith("@")
    ? spec.split("/").slice(0, 2).join("/")
    : spec.split("/")[0];
const builtin = (spec: string) => spec.startsWith("node:");

const problems: string[] = [];
for await (const rel of new Glob("**/*.{js,d.ts}").scan({ cwd: dist })) {
  const code = readFileSync(join(dist, rel), "utf8");
  for (const [, spec] of code.matchAll(
    /(?:from|import)\s*\(?\s*["']([^"'.][^"']*)["']/g,
  )) {
    if (!builtin(spec) && !declared.has(packageOf(spec))) {
      problems.push(`${rel} imports undeclared "${spec}"`);
    }
  }
}
if (problems.length > 0) {
  console.error(`build-package: ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log(`build-package: built ${pkg.name} into dist/`);
