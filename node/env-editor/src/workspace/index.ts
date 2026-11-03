// The workspace editor: several environments sharing one host, edited in an
// IDE-like shell and persisted through an injected `WorkspaceStore`.

export { Breadcrumb } from "./chrome.js";
export {
  diffSelections,
  isEmptyDiff,
  type KnobChange,
  type ListDiff,
  type SelectionDiff,
} from "./diff.js";
export { EnvTree, type EnvTreeProps } from "./EnvTree.js";
export {
  closeTab,
  type EditorTab,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  tabKey,
} from "./editors.js";
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
} from "./model.js";
export {
  createLocalStorageWorkspaceStore,
  createMemoryWorkspaceStore,
  DEFAULT_WORKSPACE_KEY,
  type KeyValueStorage,
  type MemoryStoreOptions,
  type RevisionDoc,
  type RevisionStore,
  type RevisionSummary,
  selectionDigest,
  supportsRevisions,
  type WorkspaceStore,
} from "./store.js";
export {
  gatewayUrl,
  type MemberPlan,
  useMemberPlans,
} from "./useMemberPlans.js";
export {
  starterWorkspace,
  useWorkspace,
  useWorkspaceState,
  WorkspaceProvider,
  type WorkspaceProviderProps,
  type WorkspaceRevisions,
  type WorkspaceState,
} from "./useWorkspace.js";
export { WorkspaceShell, type WorkspaceShellProps } from "./WorkspaceShell.js";
