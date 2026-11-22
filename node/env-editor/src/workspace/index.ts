// The workspace editor: several environments sharing one host, edited in an
// IDE-like shell and persisted through an injected `WorkspaceStore`.

export {
  ActivityBar,
  type ActivityItem,
  activityItems,
  BUILTIN_SIDE_VIEWS,
  type SideView,
  sidebarTabItems,
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
  type CommitAction,
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
  createEditor,
  type EditorHandle,
  EditorProvider,
  type EditorProviderProps,
  type EditorState,
  type EditorStore,
  type OpenEnvironmentOptions,
  useEditor,
  useEditorStore,
} from "./editorStore.js";
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
  ActionMenuItem,
  ActionsMenu,
  menuActions,
  RunActionButton,
  useEnvActions,
} from "./envActions.js";
export {
  EnvCard,
  type EnvCardProps,
  EnvHoverCard,
  type EnvHoverCardProps,
} from "./envCard.js";
export {
  type DraftState,
  EnvIndicator,
  EnvStatusIndicator,
  environmentIndicator,
  type Indicator,
  type IndicatorKind,
  useDraftState,
} from "./envIndicator.js";
export {
  DESIGN_TAB,
  EnvironmentPane,
  type EnvironmentPaneProps,
  EnvSubTabs,
  useEnvSubTabs,
} from "./envPane.js";
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
export { SidebarTabs } from "./SidebarTabs.js";
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
