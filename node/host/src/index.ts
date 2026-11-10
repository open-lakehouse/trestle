// `@open-lakehouse/host`: the shared host contract for Open Lakehouse service
// UIs. This entry has no React and no runtime dependencies, so plain API clients
// can use it; the React bindings are in `@open-lakehouse/host/react` and the
// fakes in `@open-lakehouse/host/testing`.

export {
  getHost,
  type HostFetchOptions,
  hostFetch,
  hostTransport,
  registerHost,
  ServiceUnavailableError,
} from "./registry.js";
export type {
  ConnectTransport,
  EnvironmentManager,
  EnvironmentRef,
  EnvironmentScope,
  FilePickerHost,
  HostKind,
  LakehouseHost,
  ModuleId,
  NotebookHost,
  NotebookSession,
  PickedFile,
  Placement,
  SecretsHost,
  ServiceBinding,
  ServiceDirectory,
  ServiceKey,
} from "./types.js";
export { serviceKey } from "./types.js";
