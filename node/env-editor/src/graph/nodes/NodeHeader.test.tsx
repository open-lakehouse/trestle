import { describe, expect, mock, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNodeDto } from "../../types";
import { type NodeSlots, NodeSlotsContext } from "../nodeSlots";

// The layout module builds an ELK instance on import, which needs a Web Worker
// bun doesn't provide; the header only reads its geometry constants.
mock.module("elkjs/lib/elk.bundled.js", () => ({ default: class {} }));
const { NodeHeader } = await import("./NodeHeader");

const node: GraphNodeDto = {
  id: "unity-catalog",
  model: "unityCatalogOSS",
  kind: "component",
  display_name: "Unity Catalog",
  summary: "Open-source catalog server.",
  category: "catalog",
  role: "data_catalog",
  placement: null,
  cloud: null,
  offers: [],
  surfaces: [],
  provisions: [],
  resource_nouns: [],
};

function render(slots?: NodeSlots) {
  return renderToStaticMarkup(
    <NodeSlotsContext.Provider value={slots}>
      <NodeHeader node={node} />
    </NodeSlotsContext.Provider>,
  );
}

describe("NodeHeader slots", () => {
  test("without slots it draws the name, role label, and native tooltip", () => {
    const html = render();
    expect(html).toContain("Unity Catalog");
    expect(html).toContain("Data catalog");
    expect(html).toContain('title="Open-source catalog server."');
    expect(html).not.toContain("nodrag");
  });

  test("slots receive the node and the default text", () => {
    const seen: string[] = [];
    const html = render({
      name: (n, fallback) => {
        seen.push(`${n.model}:${fallback}`);
        return <a href="/impl">{fallback}</a>;
      },
      role: (n, fallback) => {
        seen.push(`${n.role}:${fallback}`);
        return <a href="/capability">{fallback}</a>;
      },
    });
    expect(seen).toEqual([
      "unityCatalogOSS:Unity Catalog",
      "data_catalog:Data catalog",
    ]);
    expect(html).toContain('<a href="/impl">Unity Catalog</a>');
    expect(html).toContain('<a href="/capability">Data catalog</a>');
    expect(html).toContain('class="nodrag nopan"');
    expect(html).not.toContain('title="Open-source catalog server."');
  });

  test("a slot returning undefined keeps the default line", () => {
    const html = render({ role: () => undefined });
    expect(html).toContain("Data catalog");
    expect(html).not.toContain("nodrag");
    expect(html).toContain('title="Open-source catalog server."');
  });

  test("an aside slot draws after the name and role lines", () => {
    const html = render({
      aside: (n) => <button type="button">{n.model}: 8</button>,
    });
    expect(html).toContain('<button type="button">unityCatalogOSS: 8</button>');
    expect(html.indexOf("unityCatalogOSS: 8")).toBeGreaterThan(
      html.indexOf("Data catalog"),
    );
    expect(html).toContain('class="nodrag nopan"');
    expect(html).not.toContain('title="Open-source catalog server."');
  });

  test("an aside returning undefined draws nothing", () => {
    const html = render({ aside: () => undefined });
    expect(html).not.toContain("nodrag");
  });
});
