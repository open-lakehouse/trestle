import { describe, expect, mock, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { GraphNodeDto } from "../../types.js";
import { NodeFilesContext, type NodeFilesContextValue } from "../nodeFiles.js";
import { type NodeSlots, NodeSlotsContext } from "../nodeSlots.js";

// The layout module builds an ELK instance on import, which needs a Web Worker
// bun doesn't provide; the header only reads its geometry constants.
mock.module("elkjs/lib/elk.bundled.js", () => ({ default: class {} }));
const { NodeHeader } = await import("./NodeHeader.js");

const node: GraphNodeDto = {
  id: "unity-catalog-oss",
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
        "unity-catalog-oss": [
          "modules/unity-catalog-oss/compose.yaml",
          "modules/unity-catalog-oss/server.properties",
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
      filesContext({
        "unity-catalog-oss": ["modules/unity-catalog-oss/compose.yaml"],
      }),
    );
    expect(html).toContain('aria-label="View 1 config file"');
    expect(html.indexOf("View 1 config file")).toBeLessThan(
      html.indexOf("aside"),
    );
  });
  test("a run status is a dot on the logo, keeping the native tooltip", () => {
    const html = render({
      status: (n) =>
        n.id === "unity-catalog-oss"
          ? { state: "degraded", label: "unhealthy", detail: "2 restarts" }
          : undefined,
    });
    expect(html).toContain('data-run-state="degraded"');
    expect(html).toContain('title="unhealthy: 2 restarts"');
    expect(html).toContain('title="Open-source catalog server."');
  });

  test("no status, no dot", () => {
    expect(render({ status: () => undefined })).not.toContain("data-run-state");
  });
});

describe("NodeHeader cloud badge", () => {
  const external = (cloud: string | null): GraphNodeDto => ({
    ...node,
    kind: "external",
    placement: "external:https://br-example-unitycatalog.neon.tech",
    cloud,
  });

  test("a module run as a Neon Function names its cloud", () => {
    const html = renderToStaticMarkup(<NodeHeader node={external("neon")} />);
    expect(html).toContain(">NEON</span>");
    expect(html).toContain('title="Runs in NEON, outside the stack"');
    expect(html).toContain("lucide-cloud");
  });

  test("an unnamed cloud reads External", () => {
    const html = renderToStaticMarkup(<NodeHeader node={external("")} />);
    expect(html).toContain(">External</span>");
  });

  test("a local component has no badge", () => {
    const html = render();
    expect(html).not.toContain("lucide-cloud");
    expect(html).not.toContain("outside the stack");
  });
});
