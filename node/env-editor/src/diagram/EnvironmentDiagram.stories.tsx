import { FIXTURE_PLAN } from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react";
import { EnvironmentDiagram } from "./EnvironmentDiagram";
import { createGraphSnapshot } from "./snapshot";

// The read-only embed, drawn from a snapshot of the fixture plan, set in a
// prose-width column the way a blog post would host it.
const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph, {
  title: "A local lakehouse",
});

const meta: Meta<typeof EnvironmentDiagram> = {
  title: "EnvironmentDiagram",
  component: EnvironmentDiagram,
  args: { snapshot },
  decorators: [
    (Story) => (
      <div style={{ maxWidth: 860, margin: "2rem auto" }}>
        <Story />
      </div>
    ),
  ],
};

export default meta;

type Story = StoryObj<typeof EnvironmentDiagram>;

export const Light: Story = { args: { colorMode: "light" } };

export const Dark: Story = { args: { colorMode: "dark" } };

export const FixedHeight: Story = { args: { height: 300, legend: false } };

export const UnsupportedSnapshot: Story = {
  args: { snapshot: { format: "trestle-graph", version: 99 } },
};
