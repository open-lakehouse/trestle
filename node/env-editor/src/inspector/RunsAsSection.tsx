import { Cloud, Container } from "lucide-react";
import type { HostedResource, ModuleDto } from "../types.js";
import { fieldsOf, TextField } from "./HostedSection.js";
import { Segmented } from "./Segmented.js";

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
  return (
    <section className="border-b border-border px-4 py-3">
      <h3 className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        Where it runs
      </h3>
      <Segmented<RunsAs>
        label="Where it runs"
        value={fn ? "function" : "local"}
        options={[
          {
            value: "local",
            label: "Local container",
            icon: <Container className="h-3.5 w-3.5" />,
          },
          {
            value: "function",
            label: "Neon Function",
            icon: <Cloud className="h-3.5 w-3.5" />,
          },
        ]}
        onChange={(choice) => onChange(hostedFor(choice))}
      />
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
