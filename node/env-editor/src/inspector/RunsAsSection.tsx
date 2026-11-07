import { cn } from "@open-lakehouse/ui-kit";
import { Cloud, Container } from "lucide-react";
import type { ReactNode } from "react";
import type { HostedResource, ModuleDto } from "../types.js";
import { fieldsOf, TextField } from "./HostedSection.js";

/** The `hosted` entry that runs a service module as a Neon Function. */
export const NEON_FUNCTION: HostedResource = {
  vendor: "neon-function",
  connection: { kind: "function" },
};

/** Where a function-capable module runs. */
export type RunsAs = "local" | "function";

/** Whether `hosted` runs its module as a function. */
export function runsAsFunction(hosted: HostedResource | undefined): boolean {
  return hosted?.connection.kind === "function";
}

/** The `hosted` entry for `choice`: none for a local container. */
export function hostedFor(choice: RunsAs): HostedResource | undefined {
  return choice === "function" ? structuredClone(NEON_FUNCTION) : undefined;
}

/**
 * Where a service module that can run as a function (`module.function`)
 * runs: in its local container (no `hosted` entry, the default), or as a Neon
 * Function on the Neon branch the environment is bound to. As a function,
 * nothing of the module is rendered: no container, no helpers.
 */
export function RunsAsSection({
  module,
  hosted,
  onChange,
}: {
  module: ModuleDto;
  /** The selection's `hosted` entry for the module, if any. */
  hosted: HostedResource | undefined;
  onChange: (hosted: HostedResource | undefined) => void;
}) {
  const fn = runsAsFunction(hosted) ? hosted : undefined;
  const choose = (choice: RunsAs) => {
    if ((choice === "function") === !!fn) return;
    onChange(hostedFor(choice));
  };
  return (
    <section className="border-b border-border px-4 py-3">
      <h3 className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Where it runs
      </h3>
      <div
        role="radiogroup"
        aria-label="Where it runs"
        className="grid grid-cols-2 gap-1 rounded-md border border-border bg-muted/40 p-0.5"
      >
        <Choice
          checked={!fn}
          icon={<Container className="h-3.5 w-3.5" />}
          onSelect={() => choose("local")}
        >
          Local container
        </Choice>
        <Choice
          checked={!!fn}
          icon={<Cloud className="h-3.5 w-3.5" />}
          onSelect={() => choose("function")}
        >
          Neon Function
        </Choice>
      </div>
      {fn && (
        <div className="mt-3 flex flex-col gap-3">
          <p className="text-xs text-muted-foreground">
            Nothing of it runs locally. The environment runs on a Neon branch,
            which must already run function{" "}
            <code className="font-mono">{module.function}</code>.
          </p>
          {fieldsOf(fn.connection).map((f) => (
            <TextField
              key={f.label}
              id={`runs-as.${module.id}.${f.label}`}
              label={f.label}
              value={f.value}
              hint={f.hint}
              onChange={(value) =>
                onChange({ ...fn, connection: f.set(value) })
              }
            />
          ))}
        </div>
      )}
    </section>
  );
}

function Choice({
  checked,
  icon,
  onSelect,
  children,
}: {
  checked: boolean;
  icon: ReactNode;
  onSelect: () => void;
  children: string;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a segmented control, not native radio inputs.
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      onClick={onSelect}
      className={cn(
        "flex h-7 items-center justify-center gap-1.5 rounded-sm text-xs transition-colors focus-visible:outline-2 focus-visible:outline-ring",
        checked
          ? "bg-background font-medium text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground",
      )}
    >
      {icon}
      {children}
    </button>
  );
}
