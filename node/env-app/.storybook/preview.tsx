import { HostProvider } from "@open-lakehouse/host/react";
import { createFakeHost } from "@open-lakehouse/host/testing";
import { ThemeProvider } from "@open-lakehouse/ui-kit";
import type { Preview } from "@storybook/react-vite";
import "../src/globals.css";

// Every story renders under a fake host (a storybook-kind LakehouseHost with
// no services), the same seam the desktop shell and the web app fill. A story
// that needs services passes its own `<HostProvider host={createFakeHost(…)}>`.
const host = createFakeHost();

const preview: Preview = {
  decorators: [
    (Story) => (
      <HostProvider host={host}>
        <ThemeProvider>
          <Story />
        </ThemeProvider>
      </HostProvider>
    ),
  ],
};

export default preview;
