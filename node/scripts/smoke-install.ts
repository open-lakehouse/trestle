#!/usr/bin/env bun
// The "installs in a clean project" check for the published npm packages: pack
// each one (after `bun run build:packages`), install the tarballs into a fresh
// project outside this workspace, and typecheck + bundle a consumer that
// imports every package. Catches what the workspace hides: a missing `files`
// entry, an export pointing at an unbuilt path, a dependency only hoisted here.
//
//   bun scripts/smoke-install.ts
//
// Packing stamps versions into package.json (scripts/stamp-versions.ts); the
// script restores the originals afterwards.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { $ } from "bun";

const root = join(import.meta.dir, "..");

/** Each published package, and a consumer line that exercises it. */
const PACKAGES: Record<string, string> = {
  "ui-kit": `import { Button, cn, Textarea } from "@open-lakehouse/ui-kit";
export const kit = [Button, Textarea, cn("a", "b")];`,
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
