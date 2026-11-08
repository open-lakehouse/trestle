import { describe, expect, mock, test } from "bun:test";
import { FIXTURE_CATALOG } from "@open-lakehouse/stack-wasm/fixtures";
import { renderToStaticMarkup } from "react-dom/server";
import {
  implementationsOf,
  slotHolders,
  uniqueSlot,
} from "../selection/catalogIndex.js";
import type { HostedResource } from "../types.js";
import { implementationLabel } from "./ImplementationSection.js";

// Anything that reaches the layout module builds an ELK instance on import,
// which needs a Web Worker bun doesn't provide.
mock.module("elkjs/lib/elk.bundled.js", () => ({ default: class {} }));
const { InspectorPanel } = await import("./InspectorPanel.js");
const { NEON_FUNCTION } = await import("./RunsAsSection.js");

const noop = () => undefined;
const module = (id: string) => {
  const m = FIXTURE_CATALOG.modules.find((m) => m.id === id);
  if (!m) throw new Error(`no fixture module ${id}`);
  return m;
};

function inspect(
  moduleId: string,
  {
    hosted,
    onSwap = noop,
  }: {
    hosted?: HostedResource;
    onSwap?: (from: string, to: string) => void;
  } = {},
) {
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
      onSwap={onSwap}
      onSelect={noop}
      onOpenFile={noop}
      onClose={noop}
    />,
  );
}

describe("Implementation", () => {
  test("the fixtures carry both Unity Catalog servers, serving one API", () => {
    expect(
      implementationsOf(FIXTURE_CATALOG, module("unity-catalog-oss")).map(
        (m) => m.id,
      ),
    ).toEqual(["unity-catalog-oss", "unity-catalog-rs"]);
    expect(implementationsOf(FIXTURE_CATALOG, module("mlflow"))).toEqual([]);
    expect(implementationLabel(module("unity-catalog-rs"))).toBe("Rust");
    expect(implementationLabel(module("unity-catalog-oss"))).toBe("OSS");
  });

  test("shown for a module with sibling implementations, checked on itself", () => {
    const html = inspect("unity-catalog-oss");
    expect(html).toContain('aria-label="Implementation"');
    expect(html).toMatch(
      /role="radio" aria-checked="true"[^>]*><span[^>]*>OSS</,
    );
    expect(html).toMatch(
      /role="radio" aria-checked="false"[^>]*><span[^>]*>Rust</,
    );
    // The OSS server can't run as a function; the inspector says which can.
    expect(html).toContain("can run as a Neon Function");
    expect(html).not.toContain('aria-label="Where it runs"');
  });

  test("not shown for a module that is the only implementation", () => {
    for (const id of ["mlflow", "headwaters", "postgres"]) {
      expect(inspect(id)).not.toContain('aria-label="Implementation"');
    }
  });

  test("the Rust server offers both its implementation and where it runs", () => {
    const html = inspect("unity-catalog-rs");
    expect(html).toContain('aria-label="Implementation"');
    expect(html).toContain('aria-label="Where it runs"');
    expect(html).not.toContain("can run as a Neon Function");
  });

  test("as a function its container settings are not offered", () => {
    const local = inspect("unity-catalog-rs");
    expect(local).toContain("inspector.unity-catalog-rs.image");
    const fn = inspect("unity-catalog-rs", { hosted: NEON_FUNCTION });
    expect(fn).not.toContain("inspector.unity-catalog-rs.image");
    expect(fn).toContain("its settings don&#x27;t apply");
  });

  test("an API is a unique slot: picking the other server switches", () => {
    expect(uniqueSlot(module("unity-catalog-oss"))).toBe("api:unity-catalog");
    const holders = slotHolders(
      FIXTURE_CATALOG,
      new Set(["unity-catalog-oss"]),
    );
    expect(holders.get("unity-catalog-rs")).toBe("unity-catalog-oss");
    // Resource roles still slot as before.
    expect(uniqueSlot(module("rustfs"))).toBe("object_store");
  });
});
