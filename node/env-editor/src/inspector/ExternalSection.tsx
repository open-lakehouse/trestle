import { Input, Label, Switch } from "@open-lakehouse/ui-kit";
import { Cloud } from "lucide-react";
import type { ExternalConnection, ExternalResource, ModuleDto } from "../types";

/** An env-var reference (`${NAME}`), resolved from the environment when the
 *  stack is rendered — how a secret is written in `env.toml`. */
export const envRef = (name: string) => `\${${name}}`;

/** Whether `module` provisions a resource an external cloud can provide
 *  instead (a database, an object store). */
export function canRunExternally(module: ModuleDto | undefined): boolean {
  return (
    !!module &&
    (module.resource_kinds.includes("relational_db") ||
      module.resource_kinds.includes("object_store"))
  );
}

/** A starting external resource for `module`: example coordinates, and every
 *  secret an env-var reference resolved at render time. */
export function defaultExternal(module: ModuleDto): ExternalResource {
  if (module.resource_kinds.includes("relational_db")) {
    return {
      cloud: "aws",
      connection: {
        kind: "relational_db",
        host: "mydb.example.us-east-1.rds.amazonaws.com",
        port: 5432,
        username: "lakehouse",
        password: envRef("PG_PASSWORD"),
      },
    };
  }
  // A store whose resources are "containers" is Azure-flavoured (Azurite).
  const azure = module.resource_nouns.some(
    (n) => n.kind === "object_store" && n.singular === "container",
  );
  return azure
    ? {
        cloud: "azure",
        connection: {
          kind: "object_store",
          endpoint: "https://myaccount.blob.core.windows.net",
          uri: "abfss://{name}@myaccount.dfs.core.windows.net",
          credential: {
            flavour: "azure_blob",
            connection_string: envRef("AZURE_STORAGE_CONNECTION_STRING"),
          },
        },
      }
    : {
        cloud: "aws",
        connection: {
          kind: "object_store",
          endpoint: "https://s3.us-east-1.amazonaws.com",
          credential: {
            flavour: "s3",
            access_key_id: envRef("AWS_ACCESS_KEY_ID"),
            secret_access_key: envRef("AWS_SECRET_ACCESS_KEY"),
            region: "us-east-1",
          },
        },
      };
}

/** One editable field of an external connection. */
interface Field {
  label: string;
  value: string;
  /** Rebuild the connection with this field set to `value`. */
  set: (value: string) => ExternalConnection;
  hint?: string;
}

const SECRET_HINT = `An env-var reference like ${envRef("VAR")}, set when rendering.`;

function fieldsOf(c: ExternalConnection): Field[] {
  if (c.kind === "relational_db") {
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
  }
  const endpoint: Field = {
    label: "Endpoint",
    value: c.endpoint,
    set: (endpoint) => ({ ...c, endpoint }),
  };
  const cred = c.credential;
  if (cred.flavour === "azure_blob") {
    return [
      endpoint,
      {
        label: "URI",
        value: c.uri ?? "",
        set: (uri) => ({ ...c, uri }),
        hint: "Per resource; {name} is the container.",
      },
      {
        label: "Connection string",
        value: cred.connection_string,
        set: (connection_string) => ({
          ...c,
          credential: { ...cred, connection_string },
        }),
        hint: SECRET_HINT,
      },
    ];
  }
  return [
    endpoint,
    {
      label: "Region",
      value: cred.region,
      set: (region) => ({ ...c, credential: { ...cred, region } }),
    },
    {
      label: "Access key ID",
      value: cred.access_key_id,
      set: (access_key_id) => ({
        ...c,
        credential: { ...cred, access_key_id },
      }),
      hint: SECRET_HINT,
    },
    {
      label: "Secret access key",
      value: cred.secret_access_key,
      set: (secret_access_key) => ({
        ...c,
        credential: { ...cred, secret_access_key },
      }),
      hint: SECRET_HINT,
    },
    {
      label: "Role ARN (optional)",
      value: cred.role_arn ?? "",
      set: (role_arn) => ({
        ...c,
        credential: { ...cred, role_arn: role_arn || null },
      }),
    },
  ];
}

/**
 * Where a resource provider runs: in the stack, or provided by an external
 * cloud (a managed database, a cloud object store). When external, the stack
 * renders nothing for it and its consumers connect to the coordinates here;
 * the node stays on the canvas, framed with its cloud.
 */
export function ExternalSection({
  module,
  external,
  onChange,
}: {
  module: ModuleDto;
  external: ExternalResource | undefined;
  onChange: (external: ExternalResource | undefined) => void;
}) {
  const fieldId = (name: string) => `external.${module.id}.${name}`;
  return (
    <section className="border-b border-border px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <Label
          htmlFor={fieldId("toggle")}
          className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
        >
          <Cloud className="h-3.5 w-3.5" />
          External cloud
        </Label>
        <Switch
          id={fieldId("toggle")}
          checked={!!external}
          onCheckedChange={(on) =>
            onChange(on ? defaultExternal(module) : undefined)
          }
        />
      </div>
      {!external ? (
        <p className="mt-2 text-xs text-muted-foreground">
          Runs in the stack. Switch on when a cloud provides it instead.
        </p>
      ) : (
        <div className="mt-3 flex flex-col gap-3">
          <p className="text-xs text-muted-foreground">
            Not deployed: consumers connect here, and its{" "}
            {module.resource_kinds.includes("relational_db")
              ? "databases"
              : "buckets"}{" "}
            must already exist.
          </p>
          <TextField
            id={fieldId("cloud")}
            label="Cloud"
            value={external.cloud ?? ""}
            onChange={(cloud) =>
              onChange({ ...external, cloud: cloud || null })
            }
          />
          {fieldsOf(external.connection).map((f) => (
            <TextField
              key={f.label}
              id={fieldId(f.label)}
              label={f.label}
              value={f.value}
              hint={f.hint}
              onChange={(value) =>
                onChange({ ...external, connection: f.set(value) })
              }
            />
          ))}
        </div>
      )}
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
