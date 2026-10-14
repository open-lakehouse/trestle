import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import { transform } from "lightningcss";
import postcss, { type Rule } from "postcss";
import selectorParser from "postcss-selector-parser";
import { defineConfig, type Plugin } from "vite";

/** The class every diagram style is scoped under (the embed's root element). */
const SCOPE = "trestle-diagram";

/** Selectors standing for the document root, which the diagram root replaces. */
const ROOTS = new Set([":root", ":host", "html"]);

/** Rewrite one selector list onto the diagram root. */
const scopeSelectors = selectorParser((selectors) => {
  selectors.each((selector) => {
    const lead = selector.first;
    if (lead && ROOTS.has(lead.toString().trim())) {
      // `:root.dark` → `.trestle-diagram.dark`: only a leading root is replaced.
      lead.replaceWith(selectorParser.className({ value: SCOPE }));
      return;
    }
    selector.prepend(selectorParser.combinator({ value: " " }));
    selector.prepend(selectorParser.className({ value: SCOPE }));
  });
});

/**
 * Scope the stylesheet to the diagram root: `:root`, `:host`, and `html`
 * become `.trestle-diagram`; every other selector becomes a descendant of it.
 * Nesting is lowered first, so only flat selectors are rewritten (keyframe
 * steps are left alone).
 *
 * Cascade layers are flattened too. Unlayered styles beat layered ones, so a
 * host's layered Tailwind never wins inside the root, and the extra scope class
 * out-ranks the host's own unlayered single-class rules. Outside the root the
 * rules match nothing. Within the sheet, specificity alone keeps Tailwind's
 * order: a scoped utility (two classes) beats scoped preflight (class + element).
 */
function scopeCss(code: string): string {
  const flat = transform({
    filename: "styles.css",
    code: Buffer.from(code),
    // Old enough that CSS nesting is lowered to flat rules.
    targets: { chrome: 100 << 16, firefox: 100 << 16, safari: 15 << 16 },
  }).code.toString();

  const root = postcss.parse(flat);
  root.walkAtRules("layer", (layer) => {
    if (layer.nodes) layer.replaceWith(layer.nodes);
    else layer.remove();
  });
  root.walkRules((rule: Rule) => {
    const parent = rule.parent;
    if (
      parent?.type === "atrule" &&
      /keyframes$/.test((parent as postcss.AtRule).name)
    ) {
      return;
    }
    rule.selector = scopeSelectors.processSync(rule.selector);
  });

  return transform({
    filename: "styles.css",
    code: Buffer.from(root.toString()),
    minify: true,
  }).code.toString();
}

/** Rewrite the extracted stylesheet once Vite has emitted it. */
function scopedStyles(): Plugin {
  return {
    name: "trestle-diagram:scoped-styles",
    enforce: "post",
    generateBundle(_, bundle) {
      for (const file of Object.values(bundle)) {
        if (file.type === "asset" && file.fileName.endsWith(".css")) {
          file.source = scopeCss(String(file.source));
        }
      }
    },
  };
}

/**
 * Bundle only the private workspace packages the diagram is built from (they
 * are not published); every npm package stays an import, listed in this
 * package's `dependencies`, so the host's bundler dedupes and minifies it.
 */
function isExternal(id: string): boolean {
  return !(
    id.startsWith(".") ||
    path.isAbsolute(id) ||
    id.startsWith("@open-lakehouse/") ||
    id.startsWith("\0")
  );
}

// A library build of the read-only diagram.
export default defineConfig({
  plugins: [tailwindcss(), scopedStyles()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    lib: {
      entry: path.resolve(__dirname, "src/entry.ts"),
      formats: ["es"],
      fileName: () => "index.js",
      cssFileName: "styles",
    },
    rollupOptions: { external: isExternal },
  },
});
