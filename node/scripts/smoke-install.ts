#!/usr/bin/env bun
// The "installs in a clean project" check for the published npm packages: pack
// each one (after `bun run build:packages`), install the tarballs into a fresh
// project outside this workspace, and typecheck + bundle a consumer that
// imports every package. Catches what the workspace hides: a missing `files`
// entry, an export pointing at an unbuilt path, a dependency only hoisted here.
//
//   bun scripts/smoke-install.ts
//
// Packing stamps versions and sibling pins into package.json
// (scripts/stamp-versions.ts), and each tarball must keep those pins
// (scripts/check-packed.ts); the script restores the originals afterwards.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { $ } from "bun";

const root = join(import.meta.dir, "..");

/** Each published package, and a consumer line that exercises it. */
const PACKAGES: Record<string, string> = {
  "ui-kit": `import { Button, cn, Textarea } from "@open-lakehouse/ui-kit";
export const kit = [Button, Textarea, cn("a", "b")];`,
  host: `import { hostFetch, registerHost, type LakehouseHost } from "@open-lakehouse/host";
import { HostProvider, useService } from "@open-lakehouse/host/react";
import { createFakeHost } from "@open-lakehouse/host/testing";
const fake: LakehouseHost = createFakeHost({ services: { svc: { "/x": 1 } } });
registerHost(fake);
export const host = [hostFetch("svc"), HostProvider, useService];`,
  "stack-wasm": `import type { Planner } from "@open-lakehouse/stack-wasm/types";
import { FIXTURE_CATALOG } from "@open-lakehouse/stack-wasm/fixtures";
import { createWasmPlanner } from "@open-lakehouse/stack-wasm/stub";
export const planner: Planner = createWasmPlanner();
export const catalog = FIXTURE_CATALOG.modules.length;`,
  "env-editor": `import {
  createMemoryWorkspaceStore,
  diffSelections,
  PlannerProvider,
  supportsRevisions,
  WorkspaceProvider,
  WorkspaceShell,
  type RevisionStore,
} from "@open-lakehouse/env-editor";
import { EnvironmentDiagram } from "@open-lakehouse/env-editor/diagram";
const revisions = createMemoryWorkspaceStore(null, { revisions: true });
export const editor = [PlannerProvider, WorkspaceProvider, WorkspaceShell, EnvironmentDiagram, diffSelections];
export const keeps: boolean = supportsRevisions(revisions);
export type Store = RevisionStore;`,
};

const project = mkdtempSync(join(tmpdir(), "olai-npm-smoke-"));
const manifests = new Map<string, string>();
for (const dir of ["trestle-diagram", ...Object.keys(PACKAGES)]) {
  const path = join(root, dir, "package.json");
  manifests.set(path, readFileSync(path, "utf8"));
}

try {
  await $`bun scripts/stamp-versions.ts`.cwd(root).quiet();
  const tarballs: Record<string, string> = {};
  for (const dir of Object.keys(PACKAGES)) {
    const tgz = join(project, `${dir}.tgz`);
    await $`bun pm pack --filename ${tgz} --quiet`.cwd(join(root, dir));
    tarballs[`@open-lakehouse/${dir}`] = `file:${tgz}`;
  }
  // Installs below override the pins to the tarballs, so check the pins
  // themselves: a registry install has no such override.
  await $`bun scripts/check-packed.ts ${Object.values(tarballs).map((t) => t.slice("file:".length))}`.cwd(
    root,
  );

  writeFileSync(
    join(project, "package.json"),
    JSON.stringify(
      {
        name: "npm-smoke",
        private: true,
        type: "module",
        dependencies: {
          ...tarballs,
          react: "^19.0.0",
          "react-dom": "^19.0.0",
        },
        devDependencies: {
          "@types/react": "^19.0.0",
          "@types/react-dom": "^19.0.0",
          typescript: "^5.7.0",
          vite: "^6.0.0",
        },
        // The packages pin each other exactly; resolve those pins to the
        // tarballs too, not to a registry that may not have them yet.
        overrides: tarballs,
      },
      null,
      2,
    ),
  );
  writeFileSync(
    join(project, "tsconfig.json"),
    JSON.stringify({
      compilerOptions: {
        target: "ES2022",
        module: "esnext",
        moduleResolution: "bundler",
        jsx: "react-jsx",
        strict: true,
        noEmit: true,
        types: [],
      },
      include: ["main.tsx"],
    }),
  );
  writeFileSync(
    join(project, "main.tsx"),
    `${Object.values(PACKAGES).join("\n")}\n`,
  );
  writeFileSync(
    join(project, "index.html"),
    `<!doctype html><script type="module" src="./main.tsx"></script>`,
  );

  await $`bun install`.cwd(project);
  await $`bunx tsc -p .`.cwd(project);
  await $`bunx vite build --logLevel warn`.cwd(project);
  console.log(
    `smoke-install: ${Object.keys(tarballs).join(", ")} install and build in a clean project`,
  );
} finally {
  for (const [path, text] of manifests) writeFileSync(path, text);
  rmSync(project, { recursive: true, force: true });
}
