import { describe, expect, mock, test } from "bun:test";
import { FIXTURE_CATALOG } from "@open-lakehouse/stack-wasm/fixtures";
import { renderToStaticMarkup } from "react-dom/server";
import type { HostedResource } from "../types.js";

// Anything that reaches the layout module builds an ELK instance on import,
// which needs a Web Worker bun doesn't provide.
mock.module("elkjs/lib/elk.bundled.js", () => ({ default: class {} }));
const { InspectorPanel } = await import("./InspectorPanel.js");
const { hostedFor, NEON_FUNCTION, RunsAsSection, runsAsFunction } =
  await import("./RunsAsSection.js");

const noop = () => undefined;

function inspect(moduleId: string, hosted?: HostedResource) {
  return renderToStaticMarkup(
    <InspectorPanel
      moduleId={moduleId}
      catalog={FIXTURE_CATALOG}
      plan={null}
      overrides={{}}
      origin={{ kind: "explicit" }}
      alternatives={[]}
      settingsSeq={0}
      resourcesSeq={0}
      onAddResource={noop}
      onRemoveResource={noop}
      hosted={hosted}
      onSetHosted={noop}
      onSetKnob={noop}
      gateway={undefined}
      onSetGateway={noop}
      onRemove={noop}
      onSwap={noop}
      onSelect={noop}
      onOpenFile={noop}
      onClose={noop}
    />,
  );
}

const module = (id: string) => {
  const m = FIXTURE_CATALOG.modules.find((m) => m.id === id);
  if (!m) throw new Error(`no fixture module ${id}`);
  return m;
};

describe("Where it runs", () => {
  test("the fixtures mark the function-capable modules", () => {
    expect(module("unity-catalog-rs").function).toBe("unitycatalog");
    expect(module("headwaters").function).toBe("headwaters");
  });

  test("shown only for modules that can run as a function", () => {
    for (const id of ["unity-catalog-rs", "headwaters"]) {
      const html = inspect(id);
      expect(html).toContain("Where it runs");
      expect(html).not.toContain("Hosted connection");
    }
    // The OSS Unity Catalog can't: the Neon Function is mangrove's server.
    for (const id of ["postgres", "mlflow", "rustfs", "unity-catalog-oss"]) {
      expect(inspect(id)).not.toContain("Where it runs");
    }
  });

  test("hosted providers keep their connection form", () => {
    for (const id of [
      "hosted-postgres",
      "aws-s3",
      "azure-storage",
      "neon-storage",
    ]) {
      const html = inspect(id);
      expect(html).toContain("Hosted connection");
      expect(html).not.toContain("Where it runs");
    }
  });

  test("defaults to the local container", () => {
    const html = inspect("unity-catalog-rs");
    expect(html).toMatch(
      /role="radio" aria-checked="true"[^>]*>.*?Local container/,
    );
    expect(html).toMatch(/role="radio" aria-checked="false"[^>]*>.*?Neon/);
    expect(html).not.toContain("URL (optional)");
  });

  test("as a function it names the slug and offers the URL", () => {
    const html = inspect("unity-catalog-rs", NEON_FUNCTION);
    expect(html).toMatch(/role="radio" aria-checked="true"[^>]*>.*?Neon/);
    expect(html).toContain("Nothing of it runs locally");
    expect(html).toContain("<code");
    expect(html).toContain(">unitycatalog</code>");
    expect(html).toContain("URL (optional)");
    expect(html).toContain(
      "Defaults to the function URL the bound Neon branch supplies.",
    );
  });

  test("toggling writes and clears exactly the function entry", () => {
    expect(hostedFor("function")).toEqual({
      vendor: "neon-function",
      connection: { kind: "function" },
    });
    expect(hostedFor("local")).toBeUndefined();
    // A fresh copy: editing it never mutates the shared constant.
    expect(hostedFor("function")).not.toBe(NEON_FUNCTION);
    expect(runsAsFunction(hostedFor("function"))).toBe(true);
    expect(runsAsFunction(undefined)).toBe(false);
  });

  test("the section ignores a non-function entry", () => {
    const html = renderToStaticMarkup(
      <RunsAsSection
        module={module("unity-catalog-rs")}
        hosted={{
          connection: { kind: "s3", access_key_id: "", secret_access_key: "" },
        }}
        onChange={noop}
      />,
    );
    expect(html).toMatch(
      /role="radio" aria-checked="true"[^>]*>.*?Local container/,
    );
  });
});
