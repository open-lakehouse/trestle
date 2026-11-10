// The host contract: what an Open Lakehouse service UI may ask of whatever runs
// it (the desktop shell, a standalone web app, Storybook). It is transport-level
// only. A host says how to reach a service, keyed by its service key (the API
// it serves, else its trestle catalog module id), but knows nothing about the
// service's API beyond that name. Each service's spec, client and components
// stay in the service's own repo and version together there.

/**
 * A ConnectRPC transport, typed structurally so this package needs no
 * `@connectrpc/connect` (not even for types). Every `Transport` from
 * `@connectrpc/connect` v2 is one, and one is accepted wherever a `Transport`
 * is: `registerTransport(hostTransport("headwaters"))` typechecks.
 */
export interface ConnectTransport {
  // biome-ignore lint/suspicious/noExplicitAny: mirrors Transport#unary for any service.
  unary(...args: any[]): Promise<any>;
  // biome-ignore lint/suspicious/noExplicitAny: mirrors Transport#stream for any service.
  stream(...args: any[]): Promise<any>;
}

/** Which kind of host is running the UI. */
export type HostKind = "desktop" | "web" | "storybook";

/**
 * What a UI asks for a service by: the API it serves when its trestle module
 * declares one (the plan graph node's `api`), else its module id. Unity Catalog
 * is `"unity-catalog"` whichever server implements it (`unity-catalog-oss` or
 * `unity-catalog-rs`); Headwaters is `"headwaters"`, MLflow `"mlflow"`. A host
 * keys its directory with {@link serviceKey}.
 */
export type ServiceKey = string;

/** @deprecated The directory is keyed by {@link ServiceKey}, which is the module
 *  id only for a service that declares no API. */
export type ModuleId = ServiceKey;

/** The {@link ServiceKey} of a plan graph node: its `api`, else its `id`. */
export function serviceKey(node: {
  id: string;
  api?: string | null;
}): ServiceKey {
  return node.api ?? node.id;
}

/** Where a bound service runs, as far as the UI needs to know. */
export type Placement = "local" | "remote";

/** How a UI reaches one service. */
export interface ServiceBinding {
  /**
   * `fetch` for the service's HTTP API, with request paths relative to the
   * service root (`/api/2.1/unity-catalog/catalogs`). The host resolves the
   * origin and adds credentials, so the UI never sees base URLs or tokens.
   */
  fetch: typeof globalThis.fetch;
  /** A ConnectRPC transport, for services that speak Connect/gRPC. */
  transport?: ConnectTransport;
  /** A URL to embed the service's own UI in an iframe, where it has one. */
  embedUrl?: string;
  /** Where the service runs: a local container or a remote deployment. */
  placement: Placement;
}

/** The services the current environment binds, by {@link ServiceKey}. */
export interface ServiceDirectory {
  /** The binding for `key`, or `undefined` when the environment has none. */
  get(key: ServiceKey): ServiceBinding | undefined;
  /** The keys with a binding, where the host can list them. */
  list?(): ServiceKey[];
}

/** The environment a host is showing. */
export interface EnvironmentRef {
  /** A stable id. */
  id: string;
  /** Its display name. */
  name: string;
  /** Its workspace, where the host has workspaces. */
  workspace?: string;
  /** The revision running, where the host tracks revisions. */
  revision?: number;
}

/** The current environment, and notice of switches. */
export interface EnvironmentScope {
  /** The current environment, or `null` when none is selected. */
  current(): EnvironmentRef | null;
  /**
   * A string that changes whenever the environment (or anything the service
   * bindings depend on) changes. UIs remount on it and clear their caches:
   * see `useEnvironmentKey`.
   */
  readonly key: string;
  /** Call `onChange` after every switch. Returns an unsubscribe function. */
  subscribe(onChange: () => void): () => void;
}

/** Secrets by reference. Values live in the host's secret store (the OS
 *  keychain on desktop) and are only ever handed out on request. */
export interface SecretsHost {
  /** The value behind `ref`, or `null` when none is stored. */
  get(ref: string): Promise<string | null>;
  /** Store `value` under `ref`. */
  set(ref: string, value: string): Promise<void>;
  /** Forget `ref`. */
  delete(ref: string): Promise<void>;
}

/** An open notebook session. */
export interface NotebookSession {
  /** The iframe `src` for the notebook UI. */
  url: string;
  /** The handle to sync or close it with. */
  sessionId: string;
}

/** Opening files as live notebooks. */
export interface NotebookHost {
  /** Start (or reuse) a notebook for `path`; resolves to the session to embed. */
  openNotebook(path: string): Promise<NotebookSession>;
  /** Write the session's working copy back to its file. */
  syncNotebook(sessionId: string): Promise<void>;
  /** Close the session and release its working copy. */
  closeNotebook(sessionId: string): Promise<void>;
}

/** A file the user picked on the host's file system. */
export interface PickedFile {
  /** Its absolute path on the machine running the host. */
  path: string;
  /** Its base name, for display. */
  name: string;
}

/** A native file picker. */
export interface FilePickerHost {
  /** Ask the user for a file; `null` when they cancel. */
  pickFile(): Promise<PickedFile | null>;
}

/** Environment management, for hosts that own it (the desktop shell). */
export interface EnvironmentManager {
  /** The environments the user can switch to. */
  list(): Promise<EnvironmentRef[]>;
  /** Switch to `id`. The scope's `subscribe` callbacks fire once it is current. */
  switchTo(id: string): Promise<void>;
  /** Show the host's own environment manager, where it has one. */
  open?(): void;
}

/** Everything a host provides. Only `kind`, `environment` and `services` are
 *  required; a UI checks the optional capabilities before offering what needs
 *  them. */
export interface LakehouseHost {
  kind: HostKind;
  environment: EnvironmentScope;
  services: ServiceDirectory;
  secrets?: SecretsHost;
  notebooks?: NotebookHost;
  files?: FilePickerHost;
  shell?: EnvironmentManager;
}
