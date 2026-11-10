// Fakes for tests and Storybook: a complete `LakehouseHost` in memory, with
// services answered by handlers or fixed JSON, and an environment you can
// switch from the test.

import type {
  EnvironmentRef,
  EnvironmentScope,
  HostKind,
  LakehouseHost,
  Placement,
  ServiceBinding,
  ServiceKey,
} from "./types.js";

/** Answers one service's requests. */
export type FakeHandler = (request: Request) => Response | Promise<Response>;

/** A fake service: a handler, fixed JSON by path (`"GET /api/x"` or `"/api/x"`
 *  for any method), or a complete binding. */
export type FakeService =
  | FakeHandler
  | Record<string, unknown>
  | Partial<ServiceBinding>;

export interface FakeHostOptions {
  /** Defaults to `"storybook"`. */
  kind?: HostKind;
  /** The services, by {@link ServiceKey} (`"unity-catalog"`, `"headwaters"`). */
  services?: Record<ServiceKey, FakeService>;
  /** The starting environment; defaults to `{ id: "dev", name: "dev" }`. */
  environment?: EnvironmentRef | null;
  /** Extra capabilities (secrets, notebooks, files, shell). */
  capabilities?: Pick<
    LakehouseHost,
    "secrets" | "notebooks" | "files" | "shell"
  >;
}

/** A fake host, plus controls for the test. */
export interface FakeHost extends LakehouseHost {
  /** Switch environments; bumps the key and notifies subscribers. */
  setEnvironment(environment: EnvironmentRef | null): void;
  /** Add, replace or (with `undefined`) remove one service binding. */
  setService(moduleId: ServiceKey, service: FakeService | undefined): void;
  /** Every request a fake service received, in order. */
  readonly requests: { moduleId: ServiceKey; request: Request }[];
}

/** The base every fake service's relative paths resolve against. */
const FAKE_ORIGIN = "http://fake.host";

function isBinding(service: FakeService): service is Partial<ServiceBinding> {
  return (
    typeof service === "object" &&
    service !== null &&
    ("fetch" in service || "transport" in service || "embedUrl" in service)
  );
}

/** Answer a request from a path → JSON table; 404 for anything else. */
function fromRoutes(routes: Record<string, unknown>): FakeHandler {
  return (request) => {
    const path = new URL(request.url).pathname;
    const hit = routes[`${request.method} ${path}`] ?? routes[path];
    if (hit === undefined) {
      return new Response(`no fake route for ${request.method} ${path}`, {
        status: 404,
      });
    }
    return Response.json(hit);
  };
}

/** A fake host for tests and stories. */
export function createFakeHost(options: FakeHostOptions = {}): FakeHost {
  const listeners = new Set<() => void>();
  let environment: EnvironmentRef | null =
    options.environment === undefined
      ? { id: "dev", name: "dev" }
      : options.environment;
  let generation = 0;
  const requests: FakeHost["requests"] = [];
  const bindings = new Map<ServiceKey, ServiceBinding>();

  const bind = (moduleId: ServiceKey, service: FakeService): ServiceBinding => {
    const placement: Placement = "local";
    if (isBinding(service)) {
      return {
        placement,
        fetch: () =>
          Promise.reject(new Error(`fake "${moduleId}" has no fetch`)),
        ...service,
      };
    }
    const handler =
      typeof service === "function"
        ? service
        : fromRoutes(service as Record<string, unknown>);
    return {
      placement,
      fetch: async (input, init) => {
        const request = new Request(
          typeof input === "string" ? new URL(input, FAKE_ORIGIN) : input,
          init,
        );
        requests.push({ moduleId, request: request.clone() });
        return handler(request);
      },
    };
  };
  for (const [id, service] of Object.entries(options.services ?? {})) {
    bindings.set(id, bind(id, service));
  }

  const notify = () => {
    generation += 1;
    for (const listener of [...listeners]) listener();
  };

  const scope: EnvironmentScope = {
    current: () => environment,
    get key() {
      return `${environment?.id ?? "none"}#${generation}`;
    },
    subscribe(onChange) {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
  };

  return {
    kind: options.kind ?? "storybook",
    environment: scope,
    services: {
      get: (id) => bindings.get(id),
      list: () => [...bindings.keys()],
    },
    ...options.capabilities,
    requests,
    setEnvironment(next) {
      environment = next;
      notify();
    },
    setService(id, service) {
      if (service === undefined) bindings.delete(id);
      else bindings.set(id, bind(id, service));
      notify();
    },
  };
}
