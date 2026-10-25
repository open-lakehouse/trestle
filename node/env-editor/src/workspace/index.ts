// The workspace editor: several environments sharing one host, edited in an
// IDE-like shell and persisted through an injected `WorkspaceStore`.

export { Breadcrumb } from "./chrome";
export { EnvTree, type EnvTreeProps } from "./EnvTree";
export {
  closeTab,
  type EditorTab,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
} from "./editors";
export {
  contextFor,
  type EnvDoc,
  freeName,
  isValidName,
  MAX_SLOT,
  meshNetwork,
  nameProblem,
  nextSlot,
  type PortConflict,
  portConflicts,
  projectName,
  SLOT_PORT_STRIDE,
  type WorkspaceDoc,
  workspaceToml,
} from "./model";
export {
  createLocalStorageWorkspaceStore,
  createMemoryWorkspaceStore,
  DEFAULT_WORKSPACE_KEY,
  type KeyValueStorage,
  type WorkspaceStore,
} from "./store";
export {
  gatewayUrl,
  type MemberPlan,
  useMemberPlans,
} from "./useMemberPlans";
export {
  starterWorkspace,
  useWorkspace,
  useWorkspaceState,
  WorkspaceProvider,
  type WorkspaceProviderProps,
  type WorkspaceState,
} from "./useWorkspace";
export { WorkspaceShell, type WorkspaceShellProps } from "./WorkspaceShell";
