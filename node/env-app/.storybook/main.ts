import type { StorybookConfig } from "@storybook/react-vite";

// One Storybook for the whole node/ workspace: the ui-kit primitives and the
// env-editor components, themed by the app's token values (globals.css). The
// builder reuses this app's vite.config.ts, so the `source` condition, Tailwind
// and the fixture stub for the wasm planner apply here too: stories run on
// fixtures alone, with no wasm build and no services.
const config: StorybookConfig = {
  framework: "@storybook/react-vite",
  stories: [
    "../../ui-kit/src/**/*.stories.tsx",
    "../../env-editor/src/**/*.stories.tsx",
  ],
  core: { disableTelemetry: true },
};

export default config;
