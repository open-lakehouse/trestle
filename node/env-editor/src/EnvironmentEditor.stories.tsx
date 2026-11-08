import {
  FIXTURE_CATALOG,
  FIXTURE_PLAN,
} from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { EnvironmentEditor } from "./EnvironmentEditor.js";
import { hostedFor } from "./inspector/RunsAsSection.js";
import type { GraphNodeDto, Planner, PlanResult, Selection } from "./types.js";

// A fixture planner: the real captured catalog, and a representative plan for
// any non-empty selection. Lets the editor render end-to-end with no wasm; the
// canvas's add/swap/remove actions run, but the fixture plan doesn't change.
const fixturePlanner: Planner = {
  async catalog() {
    return FIXTURE_CATALOG;
  },
  async plan(selection) {
    if (selection.modules.length === 0 && selection.capabilities.length === 0) {
      throw new Error("select at least one technology or capability");
    }
    return FIXTURE_PLAN;
  },
};

const meta: Meta<typeof EnvironmentEditor> = {
  title: "EnvironmentEditor",
  component: EnvironmentEditor,
  parameters: { layout: "fullscreen" },
};

export default meta;

type Story = StoryObj<typeof EnvironmentEditor>;

export const Default: Story = {
  render: () => (
    <div className="h-screen">
      <EnvironmentEditor planner={fixturePlanner} catalog={FIXTURE_CATALOG} />
    </div>
  ),
};

// Unity Catalog and headwaters run as Neon Functions: the selection hosts them
// as `neon-function`, and the plan places them outside the stack, in the
// "neon" cloud, with nothing of theirs rendered (as the real planner does).
// The Unity Catalog function is mangrove's server (`unity-catalog-rs`), so the
// fixture's OSS server is swapped for it.
const FUNCTIONS = ["unity-catalog-rs", "headwaters"];
const OSS = "unity-catalog-oss";
const RS = "unity-catalog-rs";
const asRs = (id: string) => (id === OSS ? RS : id);
const functionNode = (
  id: string,
  node?: Partial<GraphNodeDto>,
): GraphNodeDto => ({
  model: null,
  display_name: id,
  summary: null,
  category: null,
  role: null,
  offers: [],
  surfaces: [],
  provisions: [],
  resource_nouns: [],
  ...node,
  id,
  kind: "external",
  placement: `external:https://br-example-${id}.compute.aws.neon.tech`,
  cloud: "neon",
});
const headwaters = functionNode("headwaters", {
  display_name: "Headwaters",
  summary: "Lineage service.",
  role: "lineage",
});
const functionsPlan: PlanResult = {
  ...FIXTURE_PLAN,
  graph: {
    nodes: [
      ...FIXTURE_PLAN.graph.nodes.map((n) =>
        n.id === OSS
          ? functionNode(RS, {
              ...n,
              model: "mangrove",
              display_name: "Unity Catalog (Rust)",
            })
          : n,
      ),
      headwaters,
    ],
    // A function is reached at its URL, never through the gateway.
    edges: FIXTURE_PLAN.graph.edges
      .map((e) => ({ ...e, from: asRs(e.from), to: asRs(e.to) }))
      .filter((e) => !(e.kind === "route" && FUNCTIONS.includes(e.to))),
  },
  files: FIXTURE_PLAN.files.filter(
    (f) => !f.path.startsWith(`modules/${OSS}/`),
  ),
};
const functionsSelection: Selection = {
  ...FIXTURE_CATALOG.default_selection,
  modules: [
    ...FIXTURE_CATALOG.default_selection.modules.map(asRs),
    "headwaters",
  ],
  hosted: Object.fromEntries(
    FUNCTIONS.map((id) => [id, hostedFor("function")]),
  ) as Selection["hosted"],
};
const functionsPlanner: Planner = {
  ...fixturePlanner,
  async plan() {
    return functionsPlan;
  },
};

/** Unity Catalog and headwaters as Neon Functions: select either one to see
 *  "Where it runs" in the inspector. */
export const NeonFunctions: Story = {
  render: () => (
    <div className="h-screen">
      <EnvironmentEditor
        planner={functionsPlanner}
        catalog={FIXTURE_CATALOG}
        initialSelection={functionsSelection}
      />
    </div>
  ),
};
