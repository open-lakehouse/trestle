import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The in-browser wasm planner is OPT-IN per build: `@open-lakehouse/stack-wasm`
// imports the gitignored wasm-bindgen artifact under node/stack-wasm/pkg/
// (produced by `just build-stack-wasm`), which default builds must never resolve.
// Unless VITE_ENABLE_WASM=true we alias the whole package to its fixture-backed
// stub, so `bun install && bun run dev` works with no wasm build. Set the flag
// (after `just build-stack-wasm`) to plan live in the browser.
const WASM_ENABLED = process.env.VITE_ENABLE_WASM === "true";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Relative asset URLs so the bundle works under any server base path.
  base: "./",
  resolve: {
    // The shared workspace packages are consumed from source; force a single
    // copy of React so hooks/context work at runtime (duplicates break them).
    dedupe: ["react", "react-dom"],
    alias: WASM_ENABLED
      ? {
          // The client imports the wasm-bindgen artifact via this bare name
          // (see @open-lakehouse/stack-wasm src/pkg.d.ts).
          "stack-topology-wasm-pkg": path.resolve(
            __dirname,
            "../stack-wasm/pkg/stack_topology_wasm.js",
          ),
        }
      : {
          // Default builds ship no wasm: alias the package to its fixture stub so
          // it never resolves the gitignored artifact under node/stack-wasm/pkg/.
          "@open-lakehouse/stack-wasm": "@open-lakehouse/stack-wasm/stub",
        },
  },
  server: {
    port: 3020,
    fs: {
      // Allow serving the sibling workspace-package sources (consumed directly
      // from their `src/` via each package's `exports` map), and the wasm
      // artifact under stack-wasm/pkg/ in wasm builds.
      allow: [path.resolve(__dirname, "..")],
    },
  },
});
