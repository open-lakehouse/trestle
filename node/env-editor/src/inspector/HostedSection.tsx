import { Button, Input, Label } from "@open-lakehouse/ui-kit";
import { Cloud } from "lucide-react";
import type { HostedConnection, HostedResource, ModuleDto } from "../types";

/** An env-var reference (`${NAME}`), resolved from the environment when the
 *  stack is rendered — how a secret is written in `env.toml`. */
export const envRef = (name: string) => `\${${name}}`;

/** The env vars `${NAME}` references in `connection` read, sorted. */
export function secretRefs(connection: HostedConnection): string[] {
  const refs = new Set<string>();
  for (const value of Object.values(connection)) {
    if (typeof value !== "string") continue;
    for (const m of value.matchAll(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g)) {
      refs.add(m[1]);
    }
  }
  return [...refs].sort();
}

/** One editable field of a hosted connection. */
interface Field {
  label: string;
  value: string;
  /** Rebuild the connection with this field set to `value`. */
  set: (value: string) => HostedConnection;
  hint?: string;
}

const SECRET_HINT = `An env-var reference like ${envRef("VAR")}, set when rendering.`;

function fieldsOf(c: HostedConnection): Field[] {
  switch (c.kind) {
    case "postgres":
      return [
        { label: "Host", value: c.host, set: (host) => ({ ...c, host }) },
        {
          label: "Port",
          value: String(c.port ?? 5432),
          set: (port) => ({ ...c, port: Number(port) || 5432 }),
        },
        {
          label: "Username",
          value: c.username,
          set: (username) => ({ ...c, username }),
        },
        {
          label: "Password",
          value: c.password,
          set: (password) => ({ ...c, password }),
          hint: SECRET_HINT,
        },
      ];
    case "s3":
      return [
        {
          label: "Region",
          value: c.region ?? "us-east-1",
          set: (region) => ({ ...c, region }),
        },
        {
          label: "Endpoint (optional)",
          value: c.endpoint ?? "",
          set: (endpoint) => ({ ...c, endpoint: endpoint || null }),
          hint: "Defaults to AWS for the region; set for an S3-compatible store.",
        },
        {
          label: "Access key ID",
          value: c.access_key_id,
          set: (access_key_id) => ({ ...c, access_key_id }),
          hint: SECRET_HINT,
        },
        {
          label: "Secret access key",
          value: c.secret_access_key,
          set: (secret_access_key) => ({ ...c, secret_access_key }),
          hint: SECRET_HINT,
        },
        {
          label: "Role ARN (optional)",
          value: c.role_arn ?? "",
          set: (role_arn) => ({ ...c, role_arn: role_arn || null }),
        },
      ];
    case "azure_blob":
      return [
        {
          label: "Storage account",
          value: c.account,
          set: (account) => ({ ...c, account }),
        },
        {
          label: "Connection string",
          value: c.connection_string,
          set: (connection_string) => ({ ...c, connection_string }),
          hint: SECRET_HINT,
        },
      ];
  }
}

/**
 * How a hosted provider (a managed database, a cloud object store) is
 * reached. The stack renders nothing for it and its consumers connect to the
 * coordinates here; the node is framed with its cloud on the canvas. Until the
 * user edits it, the module's default applies (the ecosystem's usual env vars).
 */
export function HostedSection({
  module,
  hosted,
  onChange,
}: {
  module: ModuleDto;
  /** The selection's configuration; `undefined` while the default applies. */
  hosted: HostedResource | undefined;
  onChange: (hosted: HostedResource | undefined) => void;
}) {
  const fallback = module.hosted?.default;
  const current = hosted ?? fallback;
  if (!current) return null;
  const fieldId = (name: string) => `hosted.${module.id}.${name}`;
  const vars = secretRefs(current.connection);
  return (
    <section className="border-b border-border px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          <Cloud className="h-3.5 w-3.5" />
          Hosted connection
        </h3>
        {hosted && (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-xs"
            onClick={() => onChange(undefined)}
          >
            Use defaults
          </Button>
        )}
      </div>
      <div className="mt-3 flex flex-col gap-3">
        <p className="text-xs text-muted-foreground">
          Not deployed: consumers connect here, and its{" "}
          {module.resource_nouns[0]?.plural ?? "resources"} must already exist.
          {vars.length > 0 && <> Reads {vars.join(", ")} when rendering.</>}
        </p>
        <TextField
          id={fieldId("cloud")}
          label="Cloud"
          value={current.cloud ?? ""}
          onChange={(cloud) => onChange({ ...current, cloud: cloud || null })}
        />
        {fieldsOf(current.connection).map((f) => (
          <TextField
            key={f.label}
            id={fieldId(f.label)}
            label={f.label}
            value={f.value}
            hint={f.hint}
            onChange={(value) =>
              onChange({ ...current, connection: f.set(value) })
            }
          />
        ))}
      </div>
    </section>
  );
}

function TextField({
  id,
  label,
  value,
  hint,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  hint?: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <Label htmlFor={id} className="text-xs">
        {label}
      </Label>
      <Input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-8 font-mono text-xs"
      />
      {hint && <p className="text-[11px] text-muted-foreground">{hint}</p>}
    </div>
  );
}
