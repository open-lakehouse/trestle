// The workspace editor: several environments sharing one host, edited in an
// IDE-like shell and persisted through an injected `WorkspaceStore`.

export {
  ActivityBar,
  BUILTIN_SIDE_VIEWS,
  type SideView,
  sideViewItems,
} from "./ActivityBar.js";
export { Breadcrumb } from "./chrome.js";
export {
  type ActivityActionContribution,
  ContributionsProvider,
  type ContributionsProviderProps,
  type DiffViewerProps,
  type EditorContributions,
  type EnvironmentContributions,
  type EnvironmentDetail,
  type EnvironmentLayer,
  type EnvironmentStatus,
  type EnvironmentTab,
  type FileViewerProps,
  type HostAction,
  type NodeStatus,
  type PanelContribution,
  type RunState,
  type SideViewContribution,
  useContributions,
} from "./contributions.js";
export {
  describeHostedChange,
  diffFiles,
  diffSelections,
  type FileChange,
  type HostedChange,
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
  appsAvailable,
  DESIGN_TAB,
  EnvHeader,
  EnvironmentPane,
  type EnvironmentPaneProps,
  EnvSubTabs,
  useLayerTabs,
} from "./envHeader.js";
export {
  CHANGES_TAB,
  type ChangesAvailability,
  DIAGRAM_TAB,
  type EnvView,
  FILES_TAB,
  hasLayer,
  INITIAL_VIEW,
  type LayerTabs,
  layerTabs,
  resolveView,
  type SubTab,
  withFile,
  withLayer,
  withSubTab,
} from "./layers.js";
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
export { useDiffViewer, useFileViewer } from "./viewers.js";
export { WorkspaceShell, type WorkspaceShellProps } from "./WorkspaceShell.js";
