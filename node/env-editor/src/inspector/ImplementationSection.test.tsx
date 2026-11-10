import { describe, expect, mock, test } from "bun:test";
import { FIXTURE_CATALOG } from "@open-lakehouse/stack-wasm/fixtures";
import { renderToStaticMarkup } from "react-dom/server";
import {
  implementationsOf,
  replacementsOf,
  slotHolders,
  uniqueSlot,
} from "../selection/catalogIndex.js";
import type { HostedResource, PlanResult } from "../types.js";
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
    planned,
  }: {
    hosted?: HostedResource;
    onSwap?: (from: string, to: string) => void;
    /** The components in the plan's graph (no plan when omitted). */
    planned?: string[];
  } = {},
) {
  const plan = planned && {
    graph: {
      nodes: planned.map((id) => ({
        id,
        display_name: module(id).display_name,
      })),
      edges: [],
    },
    files: [],
  };
  return renderToStaticMarkup(
    <InspectorPanel
      moduleId={moduleId}
      catalog={FIXTURE_CATALOG}
      plan={(plan ?? null) as PlanResult | null}
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

  test("held in place by a component that requires this implementation", () => {
    // The planner would add the required OSS server back beside the Rust one.
    const html = inspect("unity-catalog-oss", {
      planned: ["puppygraph", "unity-catalog-oss"],
    });
    expect(html).toMatch(/role="radio" aria-checked="false" disabled=""/);
    expect(html).toContain("needs this implementation");
    expect(html).toContain(module("puppygraph").display_name ?? "puppygraph");
    // Implied only by what uses the API, it still switches.
    const free = inspect("unity-catalog-oss", {
      planned: ["mlflow", "unity-catalog-oss"],
    });
    expect(free).not.toContain('disabled=""');
    expect(free).toContain("Switching resets its settings");
  });

  test("Replace with… leaves the other implementation to the inspector", () => {
    const ids = (id: string, planned: string[] = [id]) =>
      replacementsOf(FIXTURE_CATALOG, id, new Set(planned)).map((m) => m.id);
    expect(ids("unity-catalog-oss")).not.toContain("unity-catalog-rs");
    expect(ids("unity-catalog-rs")).not.toContain("unity-catalog-oss");
    // A role without an API still lists its other providers, unplanned ones only.
    const stores = ids("rustfs");
    expect(stores.length).toBeGreaterThan(0);
    expect(stores).not.toContain("rustfs");
    expect(ids("rustfs", ["rustfs", ...stores])).toEqual([]);
    expect(replacementsOf(undefined, "rustfs", new Set())).toEqual([]);
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
