// The late-binding registry. A host calls `registerHost` before (or after) the
// UI is imported. Clients take `hostFetch(id)` / `hostTransport(id)`, which look
// up the host and the binding again on every call, so neither the order of
// registration and client construction nor an environment switch needs a client
// to be rebuilt.

import type {
  ConnectTransport,
  LakehouseHost,
  ModuleId,
  ServiceBinding,
} from "./types.js";

let current: LakehouseHost | null = null;

/** Install `host` (or clear it with `null`). Later calls replace it. */
export function registerHost(host: LakehouseHost | null): void {
  current = host;
}

/** The registered host, or `null` when none is. */
export function getHost(): LakehouseHost | null {
  return current;
}

/** Raised when a UI calls a service the host doesn't bind. */
export class ServiceUnavailableError extends Error {
  readonly moduleId: ModuleId;
  constructor(moduleId: ModuleId, reason: string) {
    super(`service "${moduleId}" is unavailable: ${reason}`);
    this.name = "ServiceUnavailableError";
    this.moduleId = moduleId;
  }
}

/** The binding for `moduleId` in the registered host, or why there is none. */
function lookup(moduleId: ModuleId): ServiceBinding | string {
  if (!current) return "no host is registered";
  return (
    current.services.get(moduleId) ?? "the current environment doesn't bind it"
  );
}

/** Options for {@link hostFetch}. */
export interface HostFetchOptions {
  /**
   * The fetch to use when there is no host or no binding, e.g.
   * `globalThis.fetch` for a UI that also runs standalone against its own
   * origin. Without it, such a call rejects with {@link ServiceUnavailableError}.
   */
  fallback?: typeof globalThis.fetch;
}

/**
 * A `fetch` for `moduleId` that resolves the registered host's binding on every
 * call. Hand it to a client once, at any time:
 * `setDefaultUnityCatalogFetch(hostFetch("unity-catalog"))`.
 */
export function hostFetch(
  moduleId: ModuleId,
  { fallback }: HostFetchOptions = {},
): typeof globalThis.fetch {
  return (input, init) => {
    const binding = lookup(moduleId);
    if (typeof binding !== "string") return binding.fetch(input, init);
    if (fallback) return fallback(input, init);
    return Promise.reject(new ServiceUnavailableError(moduleId, binding));
  };
}

/** The transport for `moduleId` now, or the error to raise. */
function transportOf(moduleId: ModuleId): ConnectTransport {
  const binding = lookup(moduleId);
  if (typeof binding === "string") {
    throw new ServiceUnavailableError(moduleId, binding);
  }
  if (!binding.transport) {
    throw new ServiceUnavailableError(
      moduleId,
      "its binding has no ConnectRPC transport",
    );
  }
  return binding.transport;
}

/**
 * A ConnectRPC `Transport` for `moduleId` that resolves the registered host's
 * binding on every call: `registerTransport(hostTransport("headwaters"))`.
 * Calls without a bound transport reject with {@link ServiceUnavailableError}.
 */
export function hostTransport(moduleId: ModuleId): ConnectTransport {
  return {
    async unary(...args) {
      return transportOf(moduleId).unary(...args);
    },
    async stream(...args) {
      return transportOf(moduleId).stream(...args);
    },
  };
}
