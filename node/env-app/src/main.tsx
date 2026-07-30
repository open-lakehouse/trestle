import { EnvironmentEditor, PlannerProvider } from "@open-lakehouse/env-editor";
import { createWasmPlanner } from "@open-lakehouse/stack-wasm";
import { ThemeProvider, Toaster } from "@open-lakehouse/ui-kit";
import { StrictMode, useMemo } from "react";
import { createRoot } from "react-dom/client";
import { AppShell } from "./AppShell";
import "./globals.css";

// The planner resolves to the real wasm engine when built with
// VITE_ENABLE_WASM=true, or to the fixture stub otherwise (see vite.config.ts).
function App() {
  const planner = useMemo(() => createWasmPlanner(), []);
  return (
    <ThemeProvider>
      <PlannerProvider planner={planner}>
        <AppShell>
          <EnvironmentEditor colorMode="system" />
        </AppShell>
      </PlannerProvider>
      <Toaster />
    </ThemeProvider>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
