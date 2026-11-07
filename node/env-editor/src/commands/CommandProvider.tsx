// The command registry in React: components register their commands for as
// long as they're mounted (`useCommands`), one window listener answers every
// shortcut (`dispatchKey`), and ⌘K opens the palette over all of them. A
// host's commands come first, so its bindings win on a shared key; a host
// that handles a key itself calls `preventDefault`, and the editor's binding
// then stays quiet.

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { CommandPalette } from "./CommandPalette.js";
import { type Command, dispatchKey } from "./registry.js";

interface Registry {
  /** Register `commands` (a host's first); returns the unregister. */
  register(
    commands: readonly Command[],
    options?: { host?: boolean },
  ): () => void;
  /** Every registered command, a host's first. */
  all(): Command[];
  /** Open the ⌘K palette. */
  openPalette(): void;
}

const RegistryContext = createContext<Registry | null>(null);

export interface CommandProviderProps {
  /** A host's commands, merged in under their own sections. */
  commands?: readonly Command[];
  children: ReactNode;
}

/**
 * The command registry and its ⌘K palette. Nested inside another, it adds its
 * `commands` to the outer one and keeps a single palette and listener.
 */
export function CommandProvider({ commands, children }: CommandProviderProps) {
  const parent = useContext(RegistryContext);
  if (parent) {
    return (
      <>
        <HostCommands registry={parent} commands={commands} />
        {children}
      </>
    );
  }
  return <RootProvider commands={commands}>{children}</RootProvider>;
}

function HostCommands({
  registry,
  commands,
}: {
  registry: Registry;
  commands: readonly Command[] | undefined;
}) {
  useEffect(
    () => (commands ? registry.register(commands, { host: true }) : undefined),
    [registry, commands],
  );
  return null;
}

function RootProvider({ commands, children }: CommandProviderProps) {
  const entries = useRef(
    new Map<symbol, { host: boolean; commands: readonly Command[] }>(),
  );
  const [version, setVersion] = useState(0);
  const [open, setOpen] = useState(false);

  const registry = useMemo<Registry>(
    () => ({
      register(list, options) {
        const id = Symbol();
        entries.current.set(id, { host: !!options?.host, commands: list });
        setVersion((v) => v + 1);
        return () => {
          entries.current.delete(id);
          setVersion((v) => v + 1);
        };
      },
      all() {
        const values = [...entries.current.values()];
        return [
          ...values.filter((e) => e.host).flatMap((e) => e.commands),
          ...values.filter((e) => !e.host).flatMap((e) => e.commands),
        ];
      },
      openPalette: () => setOpen(true),
    }),
    [],
  );

  useEffect(
    () => (commands ? registry.register(commands, { host: true }) : undefined),
    [registry, commands],
  );

  // ⌘K itself, here so that every registry has it.
  const paletteCommand = useMemo<Command[]>(
    () => [
      {
        id: "palette.open",
        title: "Show all commands",
        section: "View",
        shortcut: "mod+k",
        allowInInputs: true,
        hidden: true,
        run: () => setOpen(true),
      },
    ],
    [],
  );
  useEffect(
    () => registry.register(paletteCommand),
    [registry, paletteCommand],
  );

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      dispatchKey(e, registry.all());
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [registry]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` signals a registration.
  const listed = useMemo(() => (open ? registry.all() : []), [open, version]);

  return (
    <RegistryContext.Provider value={registry}>
      {children}
      <CommandPalette open={open} onOpenChange={setOpen} commands={listed} />
    </RegistryContext.Provider>
  );
}

/**
 * Register `commands` while the calling component is mounted. Pass a memoized
 * list: a new array re-registers. Outside a `CommandProvider` it does nothing.
 */
export function useCommands(commands: readonly Command[]) {
  const registry = useContext(RegistryContext);
  useEffect(() => registry?.register(commands), [registry, commands]);
}

/** Open the ⌘K palette, if there is a `CommandProvider` above. */
export function useOpenPalette(): () => void {
  const registry = useContext(RegistryContext);
  return useCallback(() => registry?.openPalette(), [registry]);
}
