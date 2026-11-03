#!/usr/bin/env bun
// Stamp every publishable package's `version` from its release placeholder
// crate, the version source of truth (release-plz bumps the crate, never
// package.json). Run before `bun pm pack`: packing rewrites a `workspace:*`
// dependency to the dependency's package.json version, so all of them must be
// current, not only the package being packed.
//
//   bun scripts/stamp-versions.ts            # stamp them all
//   bun scripts/stamp-versions.ts --check    # just print the versions
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dir, "..");

/** The placeholder crate behind each npm package directory. */
const PACKAGES: Record<string, string> = {
  "trestle-diagram": "Cargo.toml",
  "ui-kit": "ui-kit/Cargo.toml",
};

const check = process.argv.includes("--check");
for (const [dir, manifest] of Object.entries(PACKAGES)) {
  const toml = readFileSync(join(root, manifest), "utf8");
  const version = /^version = "([^"]+)"$/m.exec(toml)?.[1];
  if (!version) throw new Error(`no version in node/${manifest}`);
  const pkgPath = join(root, dir, "package.json");
  if (!existsSync(pkgPath)) throw new Error(`no node/${dir}/package.json`);
  const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
  console.log(`${pkg.name}@${version}`);
  if (check) continue;
  pkg.version = version;
  writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`);
}
