import { describe, expect, test } from "bun:test";
import {
  type Command,
  dispatchKey,
  EDITOR_SHORTCUTS,
  filterCommands,
  formatShortcut,
  isEditableTarget,
  type KeyEventLike,
  matchesShortcut,
  shouldHandle,
} from "./registry.js";

/** A keydown, as the registry sees it. */
function key(
  k: string,
  opts: Partial<Omit<KeyEventLike, "preventDefault">> = {},
): KeyEventLike & { prevented: boolean } {
  const e = {
    key: k,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    defaultPrevented: false,
    target: null,
    prevented: false,
    ...opts,
    preventDefault() {
      e.prevented = true;
      e.defaultPrevented = true;
    },
  };
  return e;
}

/** An element-like target: a tag, and optionally inside an overlay. */
function target(tagName: string, inOverlay = false): EventTarget {
  return {
    tagName,
    isContentEditable: false,
    closest: () => (inOverlay ? {} : null),
  } as unknown as EventTarget;
}

function cmd(
  id: string,
  section: string,
  extra: Partial<Command> = {},
): Command & { runs: number } {
  const c = {
    id,
    title: id,
    section,
    runs: 0,
    ...extra,
    run: () => {
      c.runs++;
    },
  };
  return c;
}

describe("shortcuts", () => {
  test("mod is ⌘ or Ctrl, and shift counts for letters", () => {
    expect(matchesShortcut(key("z", { metaKey: true }), "mod+z")).toBe(true);
    expect(matchesShortcut(key("z", { ctrlKey: true }), "mod+z")).toBe(true);
    expect(
      matchesShortcut(key("Z", { metaKey: true, shiftKey: true }), "mod+z"),
    ).toBe(false);
    expect(
      matchesShortcut(
        key("Z", { metaKey: true, shiftKey: true }),
        "mod+shift+z",
      ),
    ).toBe(true);
    expect(matchesShortcut(key("z"), "mod+z")).toBe(false);
    // `/` is typed with shift on some layouts.
    expect(matchesShortcut(key("/", { shiftKey: true }), "/")).toBe(true);
    expect(
      matchesShortcut(key("k", { metaKey: true, altKey: true }), "mod+k"),
    ).toBe(false);
  });

  test("display: glyphs on macOS, words elsewhere", () => {
    expect(formatShortcut("mod+shift+z", true)).toBe("⇧⌘Z");
    expect(formatShortcut("mod+shift+z", false)).toBe("Ctrl+Shift+Z");
    expect(formatShortcut("escape", true)).toBe("Esc");
    expect(formatShortcut("/", false)).toBe("/");
  });

  test("the editor's own bindings are listed for hosts", () => {
    for (const s of [
      "mod+k",
      "mod+b",
      "mod+j",
      "mod+1",
      "mod+9",
      "mod+z",
      "/",
    ]) {
      expect(EDITOR_SHORTCUTS).toContain(s);
    }
  });
});

describe("dispatchKey", () => {
  test("runs the bound command and claims the event", () => {
    const undo = cmd("undo", "Edit", { shortcut: "mod+z" });
    const e = key("z", { metaKey: true });
    expect(dispatchKey(e, [undo])?.id).toBe("undo");
    expect(undo.runs).toBe(1);
    expect(e.prevented).toBe(true);
  });

  test("a host handler that called preventDefault keeps the editor's quiet", () => {
    const undo = cmd("undo", "Edit", { shortcut: "mod+z" });
    const e = key("z", { metaKey: true, defaultPrevented: true });
    expect(dispatchKey(e, [undo])).toBeUndefined();
    expect(undo.runs).toBe(0);
  });

  test("a host command on the same key wins (it is listed first)", () => {
    const host = cmd("host.logs", "Environment", { shortcut: "mod+j" });
    const editor = cmd("view.toggle-panel", "View", { shortcut: "mod+j" });
    dispatchKey(key("j", { metaKey: true }), [host, editor]);
    expect(host.runs).toBe(1);
    expect(editor.runs).toBe(0);
  });

  test("a disabled command passes the key on", () => {
    const off = cmd("a", "Edit", { shortcut: "escape", enabled: false });
    const on = cmd("b", "Edit", { shortcut: "escape" });
    const e = key("Escape");
    dispatchKey(e, [off, on]);
    expect(off.runs).toBe(0);
    expect(on.runs).toBe(1);
  });

  test("keys typed into a text field are text, unless the command allows it", () => {
    const add = cmd("add", "Edit", { shortcut: "/" });
    const palette = cmd("palette", "View", {
      shortcut: "mod+k",
      allowInInputs: true,
    });
    const input = target("INPUT");
    dispatchKey(key("/", { target: input }), [add]);
    expect(add.runs).toBe(0);
    dispatchKey(key("k", { metaKey: true, target: input }), [palette]);
    expect(palette.runs).toBe(1);
  });

  test("nothing fires from inside an open dialog, not even ⌘K", () => {
    const palette = cmd("palette", "View", {
      shortcut: "mod+k",
      allowInInputs: true,
    });
    const e = key("k", { metaKey: true, target: target("INPUT", true) });
    expect(dispatchKey(e, [palette])).toBeUndefined();
    expect(shouldHandle(e, palette)).toBe(false);
  });

  test("editable targets", () => {
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(target("DIV"))).toBe(false);
    expect(isEditableTarget(target("TEXTAREA"))).toBe(true);
    expect(isEditableTarget(target("DIV", true))).toBe(true);
  });
});

describe("filterCommands", () => {
  const commands = [
    cmd("Commit dev", "Edit"),
    cmd("Go to dev", "Go to"),
    cmd("Start dev", "Environment", { hint: "trestle up dev" }),
    cmd("Open MLflow", "Environment", { keywords: ["experiments"] }),
    cmd("Go to editor 1", "Go to", { hidden: true }),
    cmd("Toggle sidebar", "View"),
    cmd("Check for updates", "App"),
    cmd("Rebuild index", "Search"),
  ];
  const titles = (groups: [string, Command[]][]) =>
    groups.map(([s, list]) => [s, list.map((c) => c.title)]);

  test("host commands show up in their sections, in order; hidden ones don't", () => {
    expect(titles(filterCommands(commands, ""))).toEqual([
      ["Go to", ["Go to dev"]],
      ["Environment", ["Start dev", "Open MLflow"]],
      ["Edit", ["Commit dev"]],
      ["View", ["Toggle sidebar"]],
      ["App", ["Check for updates"]],
      ["Search", ["Rebuild index"]],
    ]);
  });

  test("a section prefix narrows the search", () => {
    expect(titles(filterCommands(commands, "env: dev"))).toEqual([
      ["Environment", ["Start dev"]],
    ]);
    expect(titles(filterCommands(commands, "edit:"))).toEqual([
      ["Edit", ["Commit dev"]],
    ]);
  });

  test("titles, then keywords and hints, match", () => {
    expect(titles(filterCommands(commands, "experiments"))).toEqual([
      ["Environment", ["Open MLflow"]],
    ]);
    expect(titles(filterCommands(commands, "trestle up"))).toEqual([
      ["Environment", ["Start dev"]],
    ]);
    expect(filterCommands(commands, "nothing like it")).toEqual([]);
  });
});
