import { describe, expect, test } from "bun:test";
import type { EnvironmentTab } from "./contributions.js";
import {
  INITIAL_VIEW,
  layerTabs,
  resolveView,
  withFile,
  withLayer,
  withSubTab,
} from "./layers.js";

const tab = (
  id: string,
  extra: Partial<EnvironmentTab> = {},
): EnvironmentTab => ({
  id,
  label: id,
  enabled: true,
  render: () => null,
  ...extra,
});

const ids = (tabs: readonly { id: string }[]) => tabs.map((t) => t.id);

describe("layerTabs", () => {
  test("Configure is Diagram, Files, then the host's configure tabs", () => {
    const tabs = layerTabs(
      [tab("catalog"), tab("policies", { layer: "configure" })],
      { revisions: false },
    );
    expect(ids(tabs.configure)).toEqual(["diagram", "files", "policies"]);
    // A host tab without a layer is an app.
    expect(ids(tabs.apps)).toEqual(["catalog"]);
  });

  test("Changes only where the store keeps revisions, enabled with a head", () => {
    const none = layerTabs([], { revisions: true });
    expect(ids(none.configure)).toEqual(["diagram", "files", "changes"]);
    expect(none.configure[2].enabled).toBe(false);
    expect(none.configure[2].reason).toContain("commit");
    expect(
      layerTabs([], { revisions: true, head: 1 }).configure[2].enabled,
    ).toBe(true);
  });
});

describe("resolveView", () => {
  const withApps = layerTabs(
    [tab("catalog"), tab("logs", { enabled: false })],
    { revisions: false },
  );
  const noApps = layerTabs([], { revisions: false });

  test("a fresh tab is Configure, on its diagram", () => {
    expect(resolveView(INITIAL_VIEW, withApps)).toEqual({
      layer: "configure",
      subTab: "diagram",
    });
  });

  test("Apps falls back to Configure when there are none", () => {
    const view = withLayer(INITIAL_VIEW, "apps");
    expect(resolveView(view, noApps).layer).toBe("configure");
    expect(resolveView(view, withApps)).toEqual({
      layer: "apps",
      subTab: "catalog",
    });
  });

  test("a sub-tab that went away falls back to its layer's first enabled one", () => {
    expect(
      resolveView(withSubTab(INITIAL_VIEW, "configure", "gone"), withApps)
        .subTab,
    ).toBe("diagram");
    expect(
      resolveView(withSubTab(INITIAL_VIEW, "apps", "gone"), withApps).subTab,
    ).toBe("catalog");
  });

  test("a picked tab that is now disabled stays picked, to say why", () => {
    expect(
      resolveView(withSubTab(INITIAL_VIEW, "apps", "logs"), withApps).subTab,
    ).toBe("logs");
  });

  test("the pre-0.0.7 Design id is the diagram", () => {
    expect(
      resolveView({ layer: "configure", configure: "design" }, noApps).subTab,
    ).toBe("diagram");
  });
});

describe("switching", () => {
  test("each layer keeps its sub-tab", () => {
    const view = withSubTab(
      withSubTab(INITIAL_VIEW, "configure", "files"),
      "apps",
      "catalog",
    );
    expect(withLayer(view, "configure")).toMatchObject({
      layer: "configure",
      configure: "files",
      apps: "catalog",
    });
  });

  test("opening a file goes to Configure → Files at it", () => {
    const view = withFile(withLayer(INITIAL_VIEW, "apps"), "modules/a/x.yaml");
    expect(view).toMatchObject({
      layer: "configure",
      configure: "files",
      filePath: "modules/a/x.yaml",
    });
    expect(
      resolveView(view, layerTabs([tab("catalog")], { revisions: false })),
    ).toEqual({ layer: "configure", subTab: "files" });
  });
});
