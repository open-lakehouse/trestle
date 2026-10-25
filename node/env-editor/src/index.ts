// Public surface of `@open-lakehouse/env-editor`: the headless environment
// editor plus its planner seam, state hook, diagram canvas and its editing
// pieces, step components, and the read-only embeddable diagram — so a host
// can use the whole editor or compose the pieces. All planning flows through the
// injected `Planner` (see ./planner and DESIGN.md); this package has no runtime
// dependency on the wasm engine.

export { FilesPanel } from "./artifacts/FilesPanel";
export {
  createGraphSnapshot,
  type DiagramImageFormat,
  type DiagramImageOptions,
  EnvironmentDiagram,
  type EnvironmentDiagramProps,
  GRAPH_SNAPSHOT_EXTENSION,
  GRAPH_SNAPSHOT_FORMAT,
  GRAPH_SNAPSHOT_VERSION,
  type GraphSnapshot,
  parseGraphSnapshot,
  stringifyGraphSnapshot,
  useDiagramImage,
  useDiagramReady,
} from "./diagram";
export {
  EnvironmentEditor,
  type EnvironmentEditorProps,
} from "./EnvironmentEditor";
export {
  CanvasNotice,
  type CanvasNoticeProps,
  CanvasToolbar,
  type CanvasToolbarProps,
} from "./graph/CanvasToolbar";
export {
  ComponentPalette,
  type ComponentPaletteProps,
} from "./graph/ComponentPalette";
export type { CanvasActions } from "./graph/canvasActions";
export {
  MarkitectureCanvas,
  type MarkitectureCanvasProps,
} from "./graph/MarkitectureCanvas";
export {
  type IconProps,
  ModuleIcon,
  type ModuleIconProps,
  projectIcon,
} from "./graph/projectIcons";
export { roleStyle } from "./graph/roleTheme";
export {
  InspectorPanel,
  type InspectorPanelProps,
} from "./inspector/InspectorPanel";
export { KnobsStep } from "./knobs/KnobsStep";
export {
  PlannerProvider,
  type PlannerProviderProps,
  usePlanner,
} from "./planner";
export { SelectionStep } from "./selection/SelectionStep";
export {
  componentOrigin,
  describeOrigin,
  type Origin,
} from "./state/origin";
export {
  type EnvironmentOptions,
  type EnvironmentState,
  useEnvironment,
} from "./state/useEnvironment";
export type {
  AuthDto,
  CatalogDto,
  ClusterDto,
  Endpoint,
  ExposedDto,
  ExposedKind,
  ExtraResource,
  GatewayDto,
  GraphDto,
  GraphNodeDto,
  Knob,
  KnobKind,
  ListenerDto,
  ModuleDto,
  NodeKind,
  OutputFile,
  Planner,
  PlanResult,
  PolicyDto,
  ProvisionedDto,
  RouteDto,
  Selection,
  ServiceSpec,
  SurfaceDto,
  SurfaceKind,
  TlsDto,
  TopologyEdgeDto,
  TopologyEdgeKind,
  VirtualHostDto,
} from "./types";
export * from "./workspace";
