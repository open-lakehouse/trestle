// The read-only diagram surface (`@open-lakehouse/env-editor/diagram`): embed a
// planned environment's canvas from a frozen graph snapshot, and export the
// rendered diagram as an image. Nothing here plans or edits; this entry is what
// `@open-lakehouse/trestle-diagram` bundles and publishes.

export type {
  ExposedDto,
  ExposedKind,
  GraphDto,
  GraphNodeDto,
  NodeKind,
  ProvisionedDto,
  ResourceNounDto,
  SurfaceDto,
  SurfaceKind,
  TopologyEdgeDto,
  TopologyEdgeKind,
} from "../types";
export {
  EnvironmentDiagram,
  type EnvironmentDiagramProps,
} from "./EnvironmentDiagram";
export {
  type DiagramImageFormat,
  type DiagramImageOptions,
  downloadText,
  downloadUrl,
  useDiagramImage,
  useDiagramReady,
} from "./exportImage";
export {
  createGraphSnapshot,
  edgeKey,
  GRAPH_SNAPSHOT_EXTENSION,
  GRAPH_SNAPSHOT_FORMAT,
  GRAPH_SNAPSHOT_VERSION,
  type GraphSnapshot,
  parseGraphSnapshot,
  SUPPORTED_GRAPH_SNAPSHOT_VERSIONS,
  stringifyGraphSnapshot,
} from "./snapshot";
