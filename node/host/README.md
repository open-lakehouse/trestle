# `@open-lakehouse/host`

The shared host contract for Open Lakehouse service UIs. With it, a service UI
(the Unity Catalog explorer, the lineage graph, …) runs unchanged in the desktop
shell, in a standalone web app and in Storybook. Whatever runs the UI says how to
reach each service, and the UI asks for the service by its trestle catalog module
id.

```bash
bun add @open-lakehouse/host   # or npm / pnpm
```

No runtime dependencies. `react` is an optional peer (only `./react` needs it),
and `transport` is typed structurally, so `@connectrpc/connect` isn't needed either.

## What lives where

The host is **transport-level only**. It carries no API schemas, generated
types or service components, and it never will. Each service keeps its spec,
its client and its components in its own repo, versioned together there:

| Repo | Owns | Its existing seam |
|---|---|---|
| mangrove | `openapi.yaml` → `unity-catalog-client` → `unity-catalog-ui` | `setDefaultUnityCatalogFetch(fetch)` |
| headwaters | `proto/` → `lineage-client` → `lineage-ui` | `registerTransport(transport)` |
| trestle | the env-editor and the planner DTOs | the `Planner` and `WorkspaceStore` seams |

This package standardises only that last column. Every host fills it the same
way, keyed by the trestle module id (`"unity-catalog"`, `"headwaters"`,
`"mlflow"`, …), the one piece of vocabulary the repos share.

## The contract

```ts
interface LakehouseHost {
  kind: "desktop" | "web" | "storybook";
  environment: EnvironmentScope;  // current(), key, subscribe()
  services: ServiceDirectory;     // get(moduleId) -> ServiceBinding | undefined
  secrets?: SecretsHost;          // values by reference; stored by the host
  notebooks?: NotebookHost;       // open files as live notebooks
  files?: FilePickerHost;         // a native file picker
  shell?: EnvironmentManager;     // list / switch environments
}

interface ServiceBinding {
  fetch: typeof fetch;            // paths relative to the service root
  transport?: Transport;          // ConnectRPC, where the service speaks it
  embedUrl?: string;              // the service's own UI, for an iframe
  placement: "local" | "remote";
}
```

The host resolves origins and credentials inside `fetch` and `transport`, so a
UI never sees base URLs or tokens.

## Adopting it in a service client

Keep the package's own `register*` function. It stays the explicit override, and
existing hosts keep working. Point its default at the host:

```ts
// mangrove: unity-catalog-client
import { hostFetch } from "@open-lakehouse/host";
setDefaultUnityCatalogFetch(
  hostFetch("unity-catalog", { fallback: globalThis.fetch }),
);

// headwaters: lineage-client
import { hostTransport } from "@open-lakehouse/host";
registerTransport(hostTransport("headwaters"));
```

`hostFetch` and `hostTransport` look the host and binding up again **on every
call**. You can create a client before or after the host is registered, and
after an environment switch the next call uses the new binding without
rebuilding the client.

Resolution order for a client call:

1. a client passed explicitly (`createUnityCatalogFetchClient({ fetch })`)
2. the package's own `register*` shim, when a host called it
3. the registered host's binding, via `hostFetch` / `hostTransport`
4. the `fallback` (e.g. `globalThis.fetch` against the page's own origin),
   if one was given

Without a binding or a fallback, a call rejects with `ServiceUnavailableError`
(it carries `moduleId`).

## Hosts

```ts
// main.ts of the desktop shell or a web app, before the UI is imported:
import { registerHost } from "@open-lakehouse/host";
registerHost(myHost);
await import("./app");
```

In React, `<HostProvider host>` wins over the registered host, so a story or a
preview can render a subtree against another host:

```tsx
import { HostProvider, useEnvironmentKey, useService } from "@open-lakehouse/host/react";

function CatalogPanel() {
  const uc = useService("unity-catalog");     // undefined: hide the panel
  if (!uc) return null;
  return <CatalogExplorer />;
}
```

### The remount-on-key rule

When the environment switches, everything cached for the old one is stale:
query caches, open sessions, selection state. Key the service UI on the
environment and clear its cache on the same key:

```tsx
const key = useEnvironmentKey();
useEffect(() => queryClient.clear(), [key]);
return <ServiceUi key={key} />;
```

`EnvironmentScope.key` changes on every switch, including a switch back to the
same environment.

## Testing and Storybook

```ts
import { createFakeHost } from "@open-lakehouse/host/testing";

const host = createFakeHost({
  services: {
    "unity-catalog": { "GET /api/2.1/unity-catalog/catalogs": { catalogs: [] } },
    headwaters: { transport: createRouterTransport(/* … */) },
    mlflow: (req) => new Response("ok"),
  },
});
host.setEnvironment({ id: "staging", name: "staging" }); // bumps the key
host.requests;                                           // what the fakes received
```

A service is a route table (`"METHOD /path"` or `"/path"` → JSON), a
`(Request) => Response` handler, or a partial `ServiceBinding`.
