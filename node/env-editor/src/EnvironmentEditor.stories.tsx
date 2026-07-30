import {
  FIXTURE_CATALOG,
  FIXTURE_PLAN,
} from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react";
import { EnvironmentEditor } from "./EnvironmentEditor";
import type { Planner } from "./types";

// A fixture planner: the real captured catalog, and a representative plan for
// any non-empty selection. Lets the editor render end-to-end with no wasm.
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
