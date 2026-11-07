// The ⌘K palette: every listed command, by section, searchable. A disabled
// command says why; a section prefix (`edit:`) narrows the search.

import { useEffect, useMemo, useState } from "react";
import {
  PaletteDialog,
  PaletteGroup,
  PaletteRow,
  usePaletteNav,
} from "../graph/paletteList.js";
import {
  type Command,
  filterCommands,
  formatShortcut,
  primaryShortcut,
} from "./registry.js";

export interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  commands: readonly Command[];
}

export function CommandPalette({
  open,
  onOpenChange,
  commands,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  useEffect(() => {
    if (open) setQuery("");
  }, [open]);
  const groups = useMemo(
    () => filterCommands(commands, query),
    [commands, query],
  );
  const pickable = useMemo(
    () => groups.flatMap(([, list]) => list.filter((c) => c.enabled !== false)),
    [groups],
  );
  const run = (command: Command) => {
    if (command.enabled === false) return;
    onOpenChange(false);
    // After the palette closes, so a command that opens a dialog of its own
    // (Add module…) gets the focus.
    setTimeout(command.run, 0);
  };
  const nav = usePaletteNav({
    open,
    items: pickable,
    keyOf: (c) => c.id,
    onPick: run,
  });
  return (
    <PaletteDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Commands"
      description="Search the editor's commands and run one."
      query={query}
      onQueryChange={(q) => {
        setQuery(q);
        nav.reset();
      }}
      onKeyDown={nav.onKeyDown}
      placeholder="Type a command, or a section and a colon (edit: …)"
      listRef={nav.listRef}
      hints={["↑↓ to move", "↵ to run", "section: to narrow", "esc to close"]}
    >
      <CommandList
        groups={groups}
        activeKey={nav.activeKey}
        onPick={run}
        onHover={nav.hover}
        query={query}
      />
    </PaletteDialog>
  );
}

/** The palette's rows, by section (exported for tests). */
export function CommandList({
  groups,
  activeKey,
  onPick,
  onHover,
  query,
}: {
  groups: [string, Command[]][];
  activeKey: string | undefined;
  onPick: (command: Command) => void;
  onHover: (command: Command) => void;
  query: string;
}) {
  if (groups.length === 0) {
    return (
      <p className="px-3 py-6 text-center text-sm text-muted-foreground">
        No command matches “{query}”.
      </p>
    );
  }
  return groups.map(([section, list]) => (
    <PaletteGroup key={section} title={section}>
      {list.map((command) => {
        const disabled = command.enabled === false;
        const shortcut = primaryShortcut(command);
        return (
          <PaletteRow
            key={command.id}
            itemKey={command.id}
            active={command.id === activeKey}
            disabled={disabled}
            onPick={() => onPick(command)}
            onHover={() => onHover(command)}
            title={command.title}
            subtitle={disabled ? command.reason : command.hint}
            trailing={
              shortcut ? (
                <kbd className="font-sans">{formatShortcut(shortcut)}</kbd>
              ) : undefined
            }
          />
        );
      })}
    </PaletteGroup>
  ));
}
