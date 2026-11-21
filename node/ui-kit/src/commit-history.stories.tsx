import type { Meta, StoryObj } from "@storybook/react-vite";
import { GitCompare } from "lucide-react";

import { Badge } from "./badge.js";
import { CommitHistory, type CommitHistoryEntry } from "./commit-history.js";

// A fixed "now", so relative times read the same on every render.
const NOW = new Date(2026, 10, 21, 15, 0);
const at = (days: number, hours = 0) =>
  new Date(NOW.getTime() - (days * 24 + hours) * 60 * 60 * 1000);

const ENTRIES: CommitHistoryEntry[] = [
  {
    id: "5",
    title: "Add the lineage service",
    description: "Wires the catalog's events into the lineage backend.",
    ref: "9f3c2a1b",
    refTitle: "9f3c2a1b7e4d5c6f8a9b0c1d2e3f4a5b",
    author: { name: "Ada Lovelace" },
    timestamp: at(0, 1),
    badges: <Badge variant="primary">HEAD</Badge>,
  },
  {
    id: "4",
    title: "Switch the object store to S3",
    ref: "4b8e0d2c",
    author: { name: "Grace Hopper" },
    timestamp: at(0, 4),
  },
  {
    id: "3",
    title: "Enable sign-in at the gateway",
    ref: "1a2b3c4d",
    author: { name: "Ada Lovelace" },
    timestamp: at(1, 2),
  },
  {
    id: "2",
    title: "Revision #2",
    ref: "7c6d5e4f",
    timestamp: at(9),
  },
  {
    id: "1",
    title: "Initial environment",
    ref: "0e1f2a3b",
    timestamp: at(40),
  },
];

const meta: Meta<typeof CommitHistory> = {
  title: "UI/CommitHistory",
  component: CommitHistory,
  args: { entries: ENTRIES, now: NOW, "aria-label": "History" },
  decorators: [
    (Story) => (
      <div className="max-w-2xl">
        <Story />
      </div>
    ),
  ],
};

export default meta;
type Story = StoryObj<typeof CommitHistory>;

export const Default: Story = {};

export const Selectable: Story = {
  args: { selectedId: "4", onSelect: () => {} },
};

export const WithActions: Story = {
  args: {
    onSelect: () => {},
    renderActions: (entry) => (
      <button
        type="button"
        aria-label={`Compare ${entry.id}`}
        className="rounded p-0.5 text-muted-foreground hover:text-foreground [&_svg]:h-3.5 [&_svg]:w-3.5"
      >
        <GitCompare />
      </button>
    ),
  },
};

// The narrow layout used in a 260px IDE sidebar.
export const Compact: Story = {
  args: {
    compact: true,
    onSelect: () => {},
    entries: ENTRIES.map((e) => ({
      ...e,
      badges: (
        <>
          <Badge>#{e.id}</Badge>
          {e.badges}
        </>
      ),
    })),
  },
  decorators: [
    (Story) => (
      <div className="w-[260px] bg-sidebar p-3">
        <Story />
      </div>
    ),
  ],
};

export const Undated: Story = {
  args: {
    entries: ENTRIES.map(({ timestamp: _, ...e }) => e),
  },
};

export const Ungrouped: Story = {
  args: { groupByDay: false },
};

export const Empty: Story = {
  args: { entries: [], emptyState: "No revisions yet." },
};
