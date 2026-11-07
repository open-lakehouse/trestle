import { describe, expect, test } from "bun:test";
import { FIXTURE_CATALOG } from "@open-lakehouse/stack-wasm/fixtures";
import type { Origin } from "../state/origin.js";
import { selectionEdits } from "../state/useEnvironment.js";
import type { Selection } from "../types.js";
import { paletteRows, type Row, rowIntent } from "./ComponentPalette.js";

const empty: Selection = {
  modules: [],
  capabilities: [],
  knob_overrides: {},
  extra_resources: [],
  gateway: { auth: false, policy: false },
};
const explicit = (): Origin => ({ kind: "explicit" });

/** Apply what picking `row` asks for, the way the editor does. */
function pick(selection: Selection, row: Row | undefined): Selection {
  const intent = row && rowIntent(row);
  if (!intent) throw new Error("not pickable");
  switch (intent.kind) {
    case "add":
      return selectionEdits.add(selection, intent.moduleId);
    case "remove":
      return selectionEdits.remove(selection, intent.moduleId);
    case "swap":
      return selectionEdits.swap(selection, intent.fromId, intent.toId);
    case "toggleCapability":
      return selectionEdits.toggleCapability(selection, intent.capability);
  }
}

const rows = (
  query: string,
  role?: string,
  selection: Selection = empty,
  planned = new Set<string>(),
) => paletteRows(FIXTURE_CATALOG, selection, planned, explicit, query, role);

describe("Add module…", () => {
  test("adds a module", () => {
    const { modules, capability } = rows("mlflow");
    expect(capability).toBeUndefined();
    const row = modules
      .flatMap(([, r]) => r)
      .find((r) => r.module.id === "mlflow");
    expect(rowIntent(row as Row)).toEqual({ kind: "add", moduleId: "mlflow" });
    expect(pick(empty, row).modules).toEqual(["mlflow"]);
  });

  test("lists no peer capabilities group: roles are a filter", () => {
    const { modules, capability } = rows("");
    expect(capability).toBeUndefined();
    expect(modules.map(([g]) => g)).not.toContain("Capabilities");
  });

  test("a role narrows to its providers, and 'any provider' adds the capability", () => {
    const { modules, capability } = rows("", "object_store");
    const ids = modules.flatMap(([, r]) => r.map((x) => x.module.id));
    expect(ids.length).toBeGreaterThan(1);
    expect(
      ids.every(
        (id) =>
          FIXTURE_CATALOG.modules.find((m) => m.id === id)?.provider_of ===
          "object_store",
      ),
    ).toBe(true);
    expect(capability?.capability).toBe("object_store");
    const next = pick(empty, capability);
    expect(next.capabilities).toEqual(["object_store"]);
    // Picked again, it drops the capability.
    const again = rows("", "object_store", next).capability;
    expect(again?.action.kind).toBe("remove");
    expect(pick(next, again).capabilities).toEqual([]);
  });

  test("adds a module through its role: a provider row under the filter", () => {
    const row = rows("", "experiment_tracking")
      .modules.flatMap(([, r]) => r)
      .find((r) => r.module.id === "mlflow");
    expect(pick(empty, row).modules).toEqual(["mlflow"]);
  });

  test("`role:` in the query filters too, by id or label prefix", () => {
    const { capability, modules } = rows("role:experiment");
    expect(capability?.capability).toBe("experiment_tracking");
    expect(modules.flatMap(([, r]) => r.map((x) => x.module.id))).toEqual([
      "mlflow",
    ]);
    expect(
      rows("role:object_st azur").modules.flatMap(([, r]) =>
        r.map((x) => x.module.id),
      ),
    ).toEqual(["azurite", "azure-storage"]);
  });

  test("a role already provided says by whom", () => {
    const planned = new Set(["rustfs"]);
    const { capability } = rows("", "object_store", empty, planned);
    expect(capability?.action).toEqual({
      kind: "unavailable",
      reason: "Via RustFS",
    });
    expect(rowIntent(capability as Row)).toBeUndefined();
  });

  test("a one-per-environment slot switches its provider", () => {
    const selection = selectionEdits.add(empty, "rustfs");
    const row = rows("azurite", undefined, selection, new Set(["rustfs"]))
      .modules.flatMap(([, r]) => r)
      .find((r) => r.module.id === "azurite");
    expect(rowIntent(row as Row)).toEqual({
      kind: "swap",
      fromId: "rustfs",
      toId: "azurite",
    });
    expect(pick(selection, row).modules).toEqual(["azurite"]);
  });
});
