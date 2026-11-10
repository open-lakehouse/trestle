// The editor's commands: everything the ⌘K palette lists and every keyboard
// shortcut the editor answers, in one registry. The editor registers its own
// (Go to, Edit, View); a host merges its own in under their own sections. Pure
// helpers, so the matching and search rules are testable without a DOM.

/** A command: something to run from the palette, a shortcut, or both. */
export interface Command {
  /** Unique within the registry, e.g. `edit.undo`. */
  id: string;
  /** What the palette shows, e.g. "Add module…". */
  title: string;
  /** The palette section it is listed under (see `SECTION_ORDER`). */
  section: string;
  /** More words to find it by. */
  keywords?: readonly string[];
  /**
   * Its key bindings, e.g. `"mod+k"`, `"mod+shift+z"`, `"/"`, `"escape"`.
   * `mod` is ⌘ on macOS and Ctrl elsewhere (either is accepted). The first is
   * the one shown.
   */
  shortcut?: string | readonly string[];
  /** A secondary line in the palette, e.g. the equivalent CLI command. */
  hint?: string;
  /** Defaults to `true`. A disabled command is listed but doesn't run. */
  enabled?: boolean;
  /** Why it is disabled. */
  reason?: string;
  /** A shortcut only: not listed in the palette (e.g. ⌘1…9). */
  hidden?: boolean;
  /** Its shortcut also fires while a text field has focus (⌘K does). Keys
   *  typed in an open dialog or menu never reach a shortcut. */
  allowInInputs?: boolean;
  run: () => void;
}

/** The order of the palette's sections; others follow, as first seen. */
export const SECTION_ORDER: readonly string[] = [
  "Go to",
  "Environment",
  "Edit",
  "View",
  "App",
];

/**
 * The keys the editor binds, so a host can pick others (or bind one of these
 * through its own `commands`, which take precedence).
 */
export const EDITOR_SHORTCUTS: readonly string[] = [
  "mod+k",
  "mod+b",
  "mod+j",
  "mod+1",
  "mod+2",
  "mod+z",
  "mod+shift+z",
  "mod+y",
  "/",
  "delete",
  "backspace",
  "escape",
];

/** The fields of a `KeyboardEvent` the registry reads. */
export interface KeyEventLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  defaultPrevented: boolean;
  target: EventTarget | null;
  preventDefault(): void;
}

interface Shortcut {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
}

/** Parse `"mod+shift+z"` into its parts. */
export function parseShortcut(shortcut: string): Shortcut {
  const parts = shortcut.toLowerCase().split("+");
  // A shortcut on the plus key itself ends in an empty part.
  const key = parts.at(-1) === "" ? "+" : (parts.pop() ?? "");
  return {
    mod: parts.includes("mod"),
    shift: parts.includes("shift"),
    alt: parts.includes("alt"),
    key,
  };
}

const shortcutsOf = (command: Pick<Command, "shortcut">): readonly string[] =>
  command.shortcut === undefined
    ? []
    : typeof command.shortcut === "string"
      ? [command.shortcut]
      : command.shortcut;

/** Whether `event` presses `shortcut`. Shift only counts for letters and
 *  named keys: `/` or `?` are typed with or without it, depending on the
 *  layout. */
export function matchesShortcut(event: KeyEventLike, shortcut: string) {
  const s = parseShortcut(shortcut);
  const key = event.key.toLowerCase();
  if (key !== s.key) return false;
  if ((event.metaKey || event.ctrlKey) !== s.mod) return false;
  if (event.altKey !== s.alt) return false;
  const shiftMatters = /^[a-z]$/.test(s.key) || s.key.length > 1;
  return !shiftMatters || event.shiftKey === s.shift;
}

/** Whether a page runs on macOS (for the ⌘ glyphs). */
export function isMac(): boolean {
  const nav = (globalThis as { navigator?: { platform?: string } }).navigator;
  return /mac|iphone|ipad/i.test(nav?.platform ?? "");
}

const KEY_NAMES: Record<string, string> = {
  escape: "Esc",
  delete: "Del",
  backspace: "⌫",
  enter: "↵",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

/** A shortcut for display: `⇧⌘Z` on macOS, `Ctrl+Shift+Z` elsewhere. */
export function formatShortcut(shortcut: string, mac = isMac()): string {
  const s = parseShortcut(shortcut);
  const key = KEY_NAMES[s.key] ?? s.key.toUpperCase();
  if (mac) {
    return `${s.alt ? "⌥" : ""}${s.shift ? "⇧" : ""}${s.mod ? "⌘" : ""}${key}`;
  }
  return [s.mod && "Ctrl", s.alt && "Alt", s.shift && "Shift", key]
    .filter(Boolean)
    .join("+");
}

/** The shortcut a command shows, if it has one. */
export function primaryShortcut(command: Pick<Command, "shortcut">) {
  return shortcutsOf(command)[0];
}

interface ElementLike {
  tagName?: unknown;
  isContentEditable?: boolean;
  closest?: (selector: string) => unknown;
}

const OVERLAY =
  '[role="dialog"], [role="alertdialog"], [role="listbox"], [role="menu"]';

/** Whether `target` is inside an open dialog or menu, whose keys are its own. */
export function isOverlayTarget(target: EventTarget | null): boolean {
  const el = target as ElementLike | null;
  return !!el && typeof el.closest === "function" && !!el.closest(OVERLAY);
}

/** Whether keys typed at `target` are text, not shortcuts: a form control, an
 *  editable element, or anything in an open dialog or menu. */
export function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as ElementLike | null;
  if (!el || typeof el.tagName !== "string") return false;
  if (el.isContentEditable) return true;
  if (["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return true;
  return isOverlayTarget(target);
}

/**
 * Whether `command`'s shortcut may handle `event`: not when something else
 * already handled it (`defaultPrevented`, e.g. a host's own listener), not
 * inside an open dialog or menu, and not in a text field unless the command
 * allows it.
 */
export function shouldHandle(
  event: KeyEventLike,
  command: Pick<Command, "allowInInputs">,
): boolean {
  if (event.defaultPrevented) return false;
  if (isOverlayTarget(event.target)) return false;
  return !!command.allowInInputs || !isEditableTarget(event.target);
}

/**
 * Run the first enabled command bound to `event`'s keys, in `commands`' order
 * (a host's first, so its bindings win), and claim the event. Returns the
 * command that ran.
 */
export function dispatchKey(
  event: KeyEventLike,
  commands: readonly Command[],
): Command | undefined {
  if (event.defaultPrevented) return undefined;
  for (const command of commands) {
    if (command.enabled === false) continue;
    if (!shortcutsOf(command).some((s) => matchesShortcut(event, s))) continue;
    if (!shouldHandle(event, command)) continue;
    event.preventDefault();
    command.run();
    return command;
  }
  return undefined;
}

/** `commands` grouped by section, in `SECTION_ORDER` then as first seen. */
export function groupBySection(
  commands: readonly Command[],
): [string, Command[]][] {
  const groups = new Map<string, Command[]>();
  for (const c of commands) {
    groups.set(c.section, [...(groups.get(c.section) ?? []), c]);
  }
  const rank = (s: string) => {
    const i = SECTION_ORDER.indexOf(s);
    return i < 0 ? SECTION_ORDER.length : i;
  };
  return [...groups.entries()].sort(([a], [b]) => rank(a) - rank(b));
}

/**
 * The palette's commands for `query`, by section. A leading `<section>:`
 * narrows to the sections it starts (`env: start`, `edit: commit`, `go:`);
 * the rest matches titles, keywords and sections, title prefixes first.
 * Hidden commands are never listed.
 */
export function filterCommands(
  commands: readonly Command[],
  query: string,
): [string, Command[]][] {
  let q = query.trim().toLowerCase();
  let listed = commands.filter((c) => !c.hidden);
  const prefix = /^([^:]+):\s*(.*)$/.exec(q);
  if (prefix?.[1] && prefix[2] !== undefined) {
    const head = prefix[1].trim();
    const narrowed = listed.filter((c) =>
      c.section.toLowerCase().startsWith(head),
    );
    if (narrowed.length > 0) {
      listed = narrowed;
      q = prefix[2];
    }
  }
  if (q === "") return groupBySection(listed);
  const rank = (c: Command) => {
    const title = c.title.toLowerCase();
    if (title.startsWith(q) || title.split(/\s+/).some((w) => w.startsWith(q)))
      return 0;
    if (title.includes(q)) return 1;
    const extra = [...(c.keywords ?? []), c.section, c.hint ?? ""];
    return extra.some((k) => k.toLowerCase().includes(q)) ? 2 : -1;
  };
  const ranked = listed
    .map((c, i) => ({ c, i, r: rank(c) }))
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((x) => x.c);
  return groupBySection(ranked);
}
