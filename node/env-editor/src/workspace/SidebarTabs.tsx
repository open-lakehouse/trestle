// The sidebar's view switcher, at its top (as in Cursor, or VS Code with the
// activity bar on top): one icon tab per side view, a host's end-aligned ones
// on the right, then the button that hides the sidebar. Arrow keys, Home and
// End move between the tabs.

import {
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { PanelLeftClose } from "lucide-react";
import { type KeyboardEvent, useRef } from "react";
import { formatShortcut } from "../commands/registry.js";
import { type ActivityItem, itemTitle, type SideView } from "./ActivityBar.js";
import { IconButton } from "./chrome.js";

export function SidebarTabs({
  items,
  view,
  onView,
  onHide,
  problemCount = 0,
  panelId,
}: {
  items: readonly ActivityItem[];
  /** The open side view. */
  view: SideView;
  onView: (view: SideView) => void;
  onHide: () => void;
  /** Marks the Environments tab when non-zero. */
  problemCount?: number;
  /** The id of the element showing the open view, for `aria-controls`. */
  panelId?: string;
}) {
  const list = useRef<HTMLDivElement>(null);
  const ordered = [
    ...items.filter((i) => i.align !== "end"),
    ...items.filter((i) => i.align === "end"),
  ];
  const onKeyDown = (e: KeyboardEvent) => {
    const at = ordered.findIndex((i) => i.id === view);
    const next =
      e.key === "ArrowRight"
        ? (at + 1) % ordered.length
        : e.key === "ArrowLeft"
          ? (at - 1 + ordered.length) % ordered.length
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? ordered.length - 1
              : -1;
    if (next < 0 || !ordered[next]) return;
    e.preventDefault();
    onView(ordered[next].id);
    list.current
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      [next]?.focus();
  };
  const tab = (item: ActivityItem) => {
    const selected = item.id === view;
    return (
      <Tooltip key={item.id}>
        <TooltipTrigger asChild>
          <button
            type="button"
            role="tab"
            aria-label={item.label}
            aria-selected={selected}
            aria-controls={selected ? panelId : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => onView(item.id)}
            className={cn(
              "relative inline-flex w-8 items-center justify-center border-b-2 text-muted-foreground transition-colors [&_svg]:h-4 [&_svg]:w-4",
              selected
                ? "border-primary text-foreground"
                : "border-transparent hover:text-foreground",
            )}
          >
            {item.icon}
            {item.id === "explorer" && problemCount > 0 && (
              <span className="absolute right-1 top-1.5 h-1.5 w-1.5 rounded-full bg-destructive" />
            )}
          </button>
        </TooltipTrigger>
        <TooltipContent side="bottom">{itemTitle(item)}</TooltipContent>
      </Tooltip>
    );
  };
  const end = ordered.filter((i) => i.align === "end");
  return (
    <div className="flex h-9 shrink-0 items-stretch border-b px-1">
      <div
        ref={list}
        role="tablist"
        aria-label="Sidebar views"
        className="flex min-w-0 flex-1 items-stretch gap-0.5"
        onKeyDown={onKeyDown}
      >
        {ordered.filter((i) => i.align !== "end").map(tab)}
        {end.length > 0 && <div className="flex-1" />}
        {end.map(tab)}
      </div>
      <div className="flex items-center pl-1">
        <IconButton
          label={`Hide sidebar (${formatShortcut("mod+b")})`}
          onClick={onHide}
        >
          <PanelLeftClose />
        </IconButton>
      </div>
    </div>
  );
}
