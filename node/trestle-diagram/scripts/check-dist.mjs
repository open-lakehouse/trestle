// Guard the published bundle: every package `dist/` imports must be declared
// in `dependencies` or `peerDependencies` (the workspace packages it is built
// from are bundled, so they must never appear), and the type bundle must not
// reference them either.
import { readFileSync } from "node:fs";

const pkg = JSON.parse(
  readFileSync(new URL("../package.json", import.meta.url)),
);
const declared = new Set([
  ...Object.keys(pkg.dependencies ?? {}),
  ...Object.keys(pkg.peerDependencies ?? {}),
]);

/** The package name of a bare import specifier (`@scope/name/sub` → `@scope/name`). */
const packageOf = (spec) =>
  spec.startsWith("@")
    ? spec.split("/").slice(0, 2).join("/")
    : spec.split("/")[0];

const problems = [];
for (const file of ["index.js", "index.d.ts"]) {
  const code = readFileSync(
    new URL(`../dist/${file}`, import.meta.url),
    "utf8",
  );
  for (const [, spec] of code.matchAll(
    /(?:from|import)\s*["']([^"'.][^"']*)["']/g,
  )) {
    const name = packageOf(spec);
    if (!declared.has(name))
      problems.push(`${file} imports undeclared "${spec}"`);
  }
}

if (problems.length > 0) {
  console.error(`check-dist: ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log("check-dist: every dist import is a declared dependency");
