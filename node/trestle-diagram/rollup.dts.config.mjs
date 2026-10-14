// Bundle the declarations into one `dist/index.d.ts`. The workspace packages the
// API is built from (env-editor, stack-wasm's DTO types) are not published, so
// their types are inlined; React and React Flow stay imports, resolved from the
// host's install.
import { dts } from "rollup-plugin-dts";

export default {
  input: "src/index.ts",
  output: { file: "dist/index.d.ts", format: "es" },
  external: [/^react($|\/)/, /^react-dom($|\/)/, /^@xyflow\/react($|\/)/],
  plugins: [dts({ respectExternal: true, tsconfig: "./tsconfig.json" })],
};
