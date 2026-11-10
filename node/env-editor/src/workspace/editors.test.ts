import { describe, expect, test } from "bun:test";
import {
  closeTab,
  envSubTabs,
  INITIAL_VIEW,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  resolveSubTab,
  tabKey,
  withFile,
} from "./editors.js";

const diagram = { env: "a" };
const rev1 = { env: "a", revision: 1 };
const prod = { env: "b" };

describe("editor tabs", () => {
  test("opening is idempotent and keeps order", () => {
    const tabs = openTab(openTab(openTab([], diagram), rev1), diagram);
    expect(tabs).toEqual([diagram, rev1]);
  });

  test("closing activates a neighbour", () => {
    const tabs = [diagram, rev1, prod];
    expect(neighbourAfterClose(tabs, tabKey(rev1))).toBe(tabKey(prod));
    expect(neighbourAfterClose(tabs, tabKey(prod))).toBe(tabKey(rev1));
    expect(closeTab(tabs, tabKey(rev1))).toEqual([diagram, prod]);
  });

  test("a removed environment's tabs close", () => {
    const tabs = [diagram, rev1, prod];
    expect(pruneTabs(tabs, new Set(["b"]))).toEqual([prod]);
    expect(pruneTabs(tabs, new Set(["a", "b"]))).toBe(tabs);
  });

  test("revision and comparison tabs are distinct from the environment's", () => {
    const keys = [
      diagram,
      { env: "a", revision: 1 },
      { env: "a", revision: 2 },
      { env: "a", compare: { from: 1, to: 2 } },
      { env: "a", compare: { from: 1, to: "draft" as const } },
    ].map(tabKey);
    expect(new Set(keys).size).toBe(keys.length);
    const rev = { env: "a", revision: 1 };
    expect(openTab([rev], { env: "a", revision: 1 })).toEqual([rev]);
  });
});

describe("environment sub-tabs", () => {
  const host = (id: string, enabled = true) => ({ id, label: id, enabled });

  test("Diagram, Files, then the host's tabs", () => {
    expect(
      envSubTabs([host("catalog")], { revisions: false }).map((t) => t.id),
    ).toEqual(["diagram", "files", "catalog"]);
  });

  test("Changes only where the store keeps revisions, enabled with a head", () => {
    const none = envSubTabs([], { revisions: true });
    expect(none.map((t) => t.id)).toEqual(["diagram", "files", "changes"]);
    expect(none[2].enabled).toBe(false);
    expect(none[2].reason).toContain("commit");
    expect(envSubTabs([], { revisions: true, head: 1 })[2].enabled).toBe(true);
  });

  const tabs = envSubTabs([host("catalog"), host("logs", false)], {
    revisions: false,
  });

  test("a fresh tab is on its diagram; the pre-0.0.7 Design id is too", () => {
    expect(resolveSubTab(INITIAL_VIEW, tabs)).toBe("diagram");
    expect(resolveSubTab({ subTab: "design" }, tabs)).toBe("diagram");
  });

  test("a tab that went away falls back; a disabled one stays, to say why", () => {
    expect(resolveSubTab({ subTab: "gone" }, tabs)).toBe("diagram");
    expect(resolveSubTab({ subTab: "logs" }, tabs)).toBe("logs");
  });

  test("opening a file goes to Files at it", () => {
    const view = withFile({ subTab: "catalog" }, "modules/a/x.yaml");
    expect(view).toEqual({ subTab: "files", filePath: "modules/a/x.yaml" });
    expect(resolveSubTab(view, tabs)).toBe("files");
  });
});
