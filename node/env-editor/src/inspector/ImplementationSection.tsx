import type { ModuleDto } from "../types.js";
import { Segmented } from "./Segmented.js";

/** What an implementation is called next to its siblings: the qualifier in
 *  its display name ("Unity Catalog (Rust)" → "Rust"), else its name or id. */
export function implementationLabel(module: ModuleDto): string {
  const name = module.display_name ?? module.id;
  return /\(([^)]+)\)\s*$/.exec(name)?.[1] ?? name;
}

/**
 * Which implementation of an API the component is, when several serve it
 * (`module.api`): the OSS or the Rust Unity Catalog, say. They are
 * alternatives, one per environment, so picking another swaps it in. Its
 * settings go with it: they belong to the implementation. A component that
 * requires this very implementation holds it in place: swapping would plan
 * both, and an environment runs one.
 */
export function ImplementationSection({
  module,
  implementations,
  requiredBy = [],
  onSwap,
}: {
  module: ModuleDto;
  /** Every module serving `module.api`, itself included, in catalog order. */
  implementations: ModuleDto[];
  /** The planned components that require `module` itself, by name. */
  requiredBy?: string[];
  onSwap: (fromId: string, toId: string) => void;
}) {
  if (implementations.length < 2) return null;
  const locked = requiredBy.length > 0;
  const functions = implementations.filter((m) => m.function);
  return (
    <section className="border-b border-border px-4 py-3">
      <h3 className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Implementation
      </h3>
      <Segmented
        label="Implementation"
        value={module.id}
        options={implementations.map((m) => ({
          value: m.id,
          label: implementationLabel(m),
        }))}
        onChange={(to) => onSwap(module.id, to)}
        disabled={locked}
      />
      <p className="mt-2 text-[11px] text-muted-foreground">
        {locked ? (
          <>
            {requiredBy.join(", ")} {requiredBy.length === 1 ? "needs" : "need"}{" "}
            this implementation, so it can't be switched while{" "}
            {requiredBy.length === 1 ? "that is" : "they are"} in the
            environment.
          </>
        ) : (
          <>
            Each serves the same API, so what uses it works with either.
            Switching resets its settings.
          </>
        )}
        {!locked &&
          functions.length > 0 &&
          functions.length < implementations.length &&
          !module.function && (
            <>
              {" "}
              Only {functions.map((m) => m.display_name ?? m.id).join(", ")} can
              run as a Neon Function.
            </>
          )}
      </p>
    </section>
  );
}
