import { Button, useTheme } from "@open-lakehouse/ui-kit";
import { Moon, Sun } from "lucide-react";
import type { ReactNode } from "react";

/** Minimal app chrome: a header with a theme toggle, and the editor filling the body. */
export function AppShell({ children }: { children: ReactNode }) {
  const { theme, setTheme } = useTheme();
  const isDark =
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);

  return (
    <div className="flex h-full flex-col">
      <header className="flex shrink-0 items-center justify-between border-b bg-sidebar px-4 py-2">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-foreground">
            Environment Editor
          </span>
          <span className="text-xs text-muted-foreground">
            trestle · stack-topology
          </span>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Toggle theme"
          onClick={() => setTheme(isDark ? "light" : "dark")}
        >
          {isDark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
        </Button>
      </header>
      <main className="min-h-0 flex-1">{children}</main>
    </div>
  );
}
