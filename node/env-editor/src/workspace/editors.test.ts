import { describe, expect, test } from "bun:test";
import {
  closeTab,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
} from "./editors.js";

const diagram = { env: "a" };
const compose = { env: "a", path: "compose.yaml" };
const prod = { env: "b" };

describe("editor tabs", () => {
  test("opening is idempotent and keeps order", () => {
    const tabs = openTab(openTab(openTab([], diagram), compose), diagram);
    expect(tabs).toEqual([diagram, compose]);
  });

  test("closing activates a neighbour", () => {
    const tabs = [diagram, compose, prod];
    expect(neighbourAfterClose(tabs, tabKey(compose))).toBe(tabKey(prod));
    expect(neighbourAfterClose(tabs, tabKey(prod))).toBe(tabKey(compose));
    expect(closeTab(tabs, tabKey(compose))).toEqual([diagram, prod]);
  });

  test("a removed environment's tabs close", () => {
    const tabs = [diagram, compose, prod];
    expect(pruneTabs(tabs, new Set(["b"]))).toEqual([prod]);
    expect(pruneTabs(tabs, new Set(["a", "b"]))).toBe(tabs);
  });

  test("revision and comparison tabs are distinct from the diagram and files", () => {
    const keys = [
      diagram,
      compose,
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
