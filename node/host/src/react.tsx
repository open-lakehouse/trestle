// React bindings for the host contract. `<HostProvider host>` wins over the
// registered host, so one app can render a subtree against another host
// (a story, a preview), and components that only read context run in either.

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useSyncExternalStore,
} from "react";
import { getHost } from "./registry.js";
import type { LakehouseHost, ServiceBinding, ServiceKey } from "./types.js";

const HostContext = createContext<LakehouseHost | null>(null);

export interface HostProviderProps {
  host: LakehouseHost;
  children?: ReactNode;
}

/** Provide `host` to the components below, over any registered host. */
export function HostProvider({ host, children }: HostProviderProps) {
  return <HostContext.Provider value={host}>{children}</HostContext.Provider>;
}

/** The provided host, else the registered one, else `null`. */
export function useOptionalHost(): LakehouseHost | null {
  return useContext(HostContext) ?? getHost();
}

/**
 * The provided host, else the registered one. Throws when there is neither,
 * which is a wiring bug rather than a runtime condition.
 */
export function useHost(): LakehouseHost {
  const host = useOptionalHost();
  if (!host) {
    throw new Error(
      "useHost needs a <HostProvider host> above it or a registerHost() call",
    );
  }
  return host;
}

/**
 * The binding for `key` (see {@link ServiceKey}), or `undefined` when the
 * environment has none (so a UI can hide what it can't reach). Rerenders when
 * the environment switches.
 */
export function useService(key: ServiceKey): ServiceBinding | undefined {
  const host = useHost();
  useEnvironmentKey();
  return host.services.get(key);
}

/**
 * The current environment's key. Key a subtree on it so it remounts on a
 * switch, and clear any query cache with it:
 * `<ServiceUi key={useEnvironmentKey()} />`.
 */
export function useEnvironmentKey(): string {
  const { environment } = useHost();
  // Stable per scope, so React doesn't resubscribe on every render; called as
  // a method in case the scope's `subscribe` uses `this`.
  const subscribe = useCallback(
    (onChange: () => void) => environment.subscribe(onChange),
    [environment],
  );
  const key = () => environment.key;
  return useSyncExternalStore(subscribe, key, key);
}

export { HostContext };
