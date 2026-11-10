// The workspace editor: several environments sharing one host, edited in an
// IDE-like shell and persisted through an injected `WorkspaceStore`.

export {
  ActivityBar,
  BUILTIN_SIDE_VIEWS,
  type SideView,
  sideViewItems,
} from "./ActivityBar.js";
export {
  BottomPanelProvider,
  type BottomPanelProviderProps,
  type BottomPanelState,
  type BottomPanelStore,
  createBottomPanel,
  useBottomPanel,
} from "./bottomPanel.js";
export { Breadcrumb } from "./chrome.js";
export {
  type ActivityActionContribution,
  ContributionsProvider,
  type ContributionsProviderProps,
  type DiffViewerProps,
  type EditorContributions,
  type EnvironmentContributions,
  type EnvironmentDetail,
  type EnvironmentStatus,
  type EnvironmentTab,
  type FileViewerProps,
  type HostAction,
  type LegacyPanelContribution,
  type NodeStatus,
  type PanelContribution,
  type RunState,
  type SideViewContribution,
  useContributions,
  type WorkbenchViewContribution,
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
  CHANGES_TAB,
  type ChangesAvailability,
  closeTab,
  DIAGRAM_TAB,
  type EditorTab,
  type EnvView,
  envSubTabs,
  FILES_TAB,
  INITIAL_VIEW,
  neighbourAfterClose,
  openTab,
  pruneTabs,
  resolveSubTab,
  type SubTab,
  tabKey,
  withFile,
} from "./editors.js";
export {
  DESIGN_TAB,
  EnvHeader,
  EnvironmentPane,
  type EnvironmentPaneProps,
  EnvSubTabs,
  useEnvSubTabs,
} from "./envHeader.js";
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
export {
  createWorkbench,
  EDITOR_VIEW,
  useWorkbench,
  useWorkbenchStore,
  WorkbenchArea,
  WorkbenchProvider,
  type WorkbenchProviderProps,
  type WorkbenchStore,
} from "./workbench.js";
