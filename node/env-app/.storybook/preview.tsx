import { ThemeProvider } from "@open-lakehouse/ui-kit";
import type { Preview } from "@storybook/react-vite";
import "../src/globals.css";

const preview: Preview = {
  decorators: [
    (Story) => (
      <ThemeProvider>
        <Story />
      </ThemeProvider>
    ),
  ],
};

export default preview;
