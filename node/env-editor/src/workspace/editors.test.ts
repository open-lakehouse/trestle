import { describe, expect, test } from "bun:test";
import {
  closeTab,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
} from "./editors";

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
});
