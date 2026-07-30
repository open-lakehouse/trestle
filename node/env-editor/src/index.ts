// Public surface of `@open-lakehouse/env-editor`: the headless environment
// editor plus its planner seam, diagram canvas, and step components — so a host
// can use the whole editor or compose the pieces. All planning flows through the
// injected `Planner` (see ./planner and DESIGN.md); this package has no runtime
// dependency on the wasm engine.

export { ArtifactsPanel } from "./artifacts/ArtifactsPanel";
export {
  EnvironmentEditor,
  type EnvironmentEditorProps,
} from "./EnvironmentEditor";
export {
  MarkitectureCanvas,
  type MarkitectureCanvasProps,
} from "./graph/MarkitectureCanvas";
export { roleStyle } from "./graph/roleTheme";
export { KnobsStep } from "./knobs/KnobsStep";
export {
  PlannerProvider,
  type PlannerProviderProps,
  usePlanner,
} from "./planner";
export { SelectionStep } from "./selection/SelectionStep";
export type {
  ArtifactsDto,
  CatalogDto,
  ClusterDto,
  EdgeDto,
  Endpoint,
  GatewayDto,
  GraphDto,
  GraphNodeDto,
  Knob,
  KnobKind,
  ListenerDto,
  ModuleDto,
  Planner,
  PlanResult,
  RouteDto,
  Selection,
  ServiceSpec,
} from "./types";
