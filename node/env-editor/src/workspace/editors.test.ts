import { describe, expect, test } from "bun:test";
import {
  closeTab,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
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
