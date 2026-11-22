import type { Meta, StoryObj } from "@storybook/react-vite";
import { GitCommitHorizontal, Play } from "lucide-react";

import { DropdownMenuItem } from "./dropdown-menu.js";
import { SplitButton } from "./split-button.js";

const meta: Meta<typeof SplitButton> = {
  title: "UI/SplitButton",
  component: SplitButton,
};

export default meta;
type Story = StoryObj<typeof SplitButton>;

const menu = (
  <DropdownMenuItem>
    <Play /> Commit and start
  </DropdownMenuItem>
);

export const ExtraSmall: Story = {
  render: () => (
    <div className="flex items-center gap-3">
      <SplitButton size="xs" menu={menu} menuLabel="More commit options">
        <GitCommitHorizontal /> Commit
      </SplitButton>
      <SplitButton
        size="xs"
        variant="outline"
        menu={menu}
        menuLabel="More commit options"
      >
        <GitCommitHorizontal /> Commit
      </SplitButton>
    </div>
  ),
};

export const Disabled: Story = {
  render: () => (
    <SplitButton size="xs" disabled menu={menu}>
      <GitCommitHorizontal /> Commit
    </SplitButton>
  ),
};
