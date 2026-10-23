import { describe, expect, mock, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNodeDto } from "../../types";
import { NodeFilesContext, type NodeFilesContextValue } from "../nodeFiles";
import { type NodeSlots, NodeSlotsContext } from "../nodeSlots";

// The layout module builds an ELK instance on import, which needs a Web Worker
// bun doesn't provide; the header only reads its geometry constants.
mock.module("elkjs/lib/elk.bundled.js", () => ({ default: class {} }));
const { NodeHeader } = await import("./NodeHeader");

const node: GraphNodeDto = {
  id: "unity-catalog",
  model: "unityCatalogServer",
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

function render(slots?: NodeSlots, files?: NodeFilesContextValue) {
  return renderToStaticMarkup(
    <NodeFilesContext.Provider value={files ?? null}>
      <NodeSlotsContext.Provider value={slots}>
        <NodeHeader node={node} />
      </NodeSlotsContext.Provider>
    </NodeFilesContext.Provider>,
  );
}

function filesContext(byNode: Record<string, string[]>): NodeFilesContextValue {
  return { filesOf: (id) => byNode[id] ?? [], open: () => undefined };
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
      "unityCatalogServer:Unity Catalog",
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
    expect(html).toContain(
      '<button type="button">unityCatalogServer: 8</button>',
    );
    expect(html.indexOf("unityCatalogServer: 8")).toBeGreaterThan(
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

describe("NodeHeader files chip", () => {
  test("counts the node's files on a read-only canvas", () => {
    const html = render(
      undefined,
      filesContext({
        "unity-catalog": [
          "modules/unity-catalog/compose.yaml",
          "modules/unity-catalog/server.properties",
        ],
      }),
    );
    expect(html).toContain('aria-label="View 2 config files"');
    expect(html).toContain("nodrag nopan");
    // The chip has its own tooltip; the header keeps the summary.
    expect(html).toContain('title="Open-source catalog server."');
  });

  test("a node without files has no chip", () => {
    const html = render(
      undefined,
      filesContext({ envoy: ["modules/envoy/compose.yaml"] }),
    );
    expect(html).not.toContain("config file");
  });

  test("the chip comes before a host's aside", () => {
    const html = render(
      { aside: () => <span>aside</span> },
      filesContext({ "unity-catalog": ["modules/unity-catalog/compose.yaml"] }),
    );
    expect(html).toContain('aria-label="View 1 config file"');
    expect(html.indexOf("View 1 config file")).toBeLessThan(
      html.indexOf("aside"),
    );
  });
});
