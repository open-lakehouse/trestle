#!/usr/bin/env bun
// Stamp every publishable package's `version` from its release placeholder
// crate, the version source of truth (release-plz bumps the crate, never
// package.json), and pin each `workspace:` dependency on a sibling to that
// sibling's stamped version. Run before `bun pm pack`.
//
// The pinning is what makes a published package installable: `bun pm pack`
// resolves `workspace:*` from the workspace versions recorded in bun.lock, not
// from the siblings' package.json, and bun.lock records them as `0.0.0`. So
// env-editor 0.0.4 shipped depending on `@open-lakehouse/ui-kit@0.0.0`. With
// the specs already concrete, pack has nothing left to resolve.
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
  host: "host/Cargo.toml",
  "stack-wasm": "stack-wasm/Cargo.toml",
  "env-editor": "env-editor/Cargo.toml",
};

const DEP_FIELDS = [
  "dependencies",
  "devDependencies",
  "peerDependencies",
  "optionalDependencies",
] as const;

/** A `workspace:` spec pinned to `version`: `*` exact, `^`/`~` keep their range. */
export function pinWorkspaceSpec(spec: string, version: string): string {
  const range = spec.slice("workspace:".length);
  if (range === "*" || range === "") return version;
  if (range === "^" || range === "~") return `${range}${version}`;
  // An explicit range (`workspace:^1.2.0`) is already concrete.
  return range;
}

const check = process.argv.includes("--check");
const manifests = Object.entries(PACKAGES).map(([dir, manifest]) => {
  const toml = readFileSync(join(root, manifest), "utf8");
  const version = /^version = "([^"]+)"$/m.exec(toml)?.[1];
  if (!version) throw new Error(`no version in node/${manifest}`);
  const path = join(root, dir, "package.json");
  if (!existsSync(path)) throw new Error(`no node/${dir}/package.json`);
  const pkg = JSON.parse(readFileSync(path, "utf8"));
  return { path, pkg, version };
});
const versions = new Map<string, string>(
  manifests.map(({ pkg, version }) => [pkg.name, version]),
);

for (const { path, pkg, version } of manifests) {
  console.log(`${pkg.name}@${version}`);
  if (check) continue;
  pkg.version = version;
  for (const field of DEP_FIELDS) {
    const deps: Record<string, string> | undefined = pkg[field];
    for (const [name, spec] of Object.entries(deps ?? {})) {
      if (!spec.startsWith("workspace:")) continue;
      const sibling = versions.get(name);
      if (!sibling) {
        throw new Error(
          `${pkg.name} depends on ${name} via ${spec}, which isn't a publishable package`,
        );
      }
      (deps as Record<string, string>)[name] = pinWorkspaceSpec(spec, sibling);
    }
  }
  writeFileSync(path, `${JSON.stringify(pkg, null, 2)}\n`);
}
