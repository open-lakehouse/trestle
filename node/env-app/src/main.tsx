import {
  createLocalStorageWorkspaceStore,
  PlannerProvider,
  WorkspaceProvider,
  WorkspaceShell,
} from "@open-lakehouse/env-editor";
import { createWasmPlanner } from "@open-lakehouse/stack-wasm";
import { ThemeProvider, Toaster, useTheme } from "@open-lakehouse/ui-kit";
import { StrictMode, useMemo } from "react";
import { createRoot } from "react-dom/client";
import { ThemeToggle } from "./ThemeToggle";
import "./globals.css";

// The planner resolves to the real wasm engine when built with
// VITE_ENABLE_WASM=true, or to the fixture stub otherwise (see vite.config.ts).
// The workspace lives in this browser's localStorage; another WorkspaceStore
// (a folder, a local trestle server) drops in here.
function App() {
  const planner = useMemo(() => createWasmPlanner(), []);
  const store = useMemo(() => createLocalStorageWorkspaceStore(), []);
  return (
    <ThemeProvider>
      <PlannerProvider planner={planner}>
        <WorkspaceProvider store={store}>
          <Shell />
        </WorkspaceProvider>
      </PlannerProvider>
      <Toaster />
    </ThemeProvider>
  );
}

/** The workspace shell, its canvas following the app's theme (not just the
 *  system's). */
function Shell() {
  const { theme } = useTheme();
  return <WorkspaceShell colorMode={theme} statusItems={<ThemeToggle />} />;
}

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
