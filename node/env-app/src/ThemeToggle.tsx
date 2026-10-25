import { useTheme } from "@open-lakehouse/ui-kit";
import { Moon, Sun } from "lucide-react";

/** The status-bar theme toggle: light ↔ dark (from the system preference). */
export function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const isDark =
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);

  return (
    <button
      type="button"
      aria-label="Toggle theme"
      onClick={() => setTheme(isDark ? "light" : "dark")}
      className="inline-flex h-full items-center px-2 hover:bg-foreground/10 [&_svg]:h-3.5 [&_svg]:w-3.5"
    >
      {isDark ? <Sun /> : <Moon />}
    </button>
  );
}
