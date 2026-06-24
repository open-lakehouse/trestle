import path from "node:path";

import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// In dev, proxy /v1 and /healthz to the Rust server so the generated client
// uses relative URLs that resolve the same way in dev and prod.
const APP_PORT = Number(process.env.DATABRICKS_APP_PORT ?? 8080);

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      // The generated browser client, compiled to WASM by `just build-wasm`
      // (wasm-pack `--target web`): `import init, { ...Client } from "@/wasm/client"`.
      "@/wasm": path.resolve(__dirname, "./src/wasm"),
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/v1": `http://localhost:${APP_PORT}`,
      "/healthz": `http://localhost:${APP_PORT}`,
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
});