// An environment's host actions as the shell draws them: its run action as an
// icon button (the tree row, the end of the sub-tabs) and the rest in a ⋯
// menu. See `contributions.tsx` for what a host gives.

import {
  Button,
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import { Ellipsis, Play } from "lucide-react";
import { type ReactNode, useRef } from "react";
import { type HostAction, useContributions } from "./contributions.js";

/** The actions for an environment's ⋯ menus: all of `actions` but the run
 *  action's. */
export function menuActions(
  actions: readonly HostAction[],
  run: HostAction | undefined,
): HostAction[] {
  return run ? actions.filter((a) => a.id !== run.id) : [...actions];
}

/** Environment `envId`'s run action and its menu actions, from the host. */
export function useEnvActions(envId: string | undefined): {
  run: HostAction | undefined;
  menu: HostAction[];
} {
  const env = useContributions().contributions?.environment;
  const run = envId ? env?.runAction?.(envId) : undefined;
  const actions = (envId && env?.actions?.(envId)) || [];
  return { run, menu: menuActions(actions, run) };
}

/** The run action as an icon button, its label (or why it can't run) the
 *  tooltip. */
export function RunActionButton({
  action,
  className,
}: {
  action: HostAction;
  className?: string;
}) {
  const enabled = action.enabled ?? true;
  const tip = !enabled && action.reason ? action.reason : action.label;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* A disabled button gets no pointer events: the wrapper carries the tooltip, and is reachable by keyboard. */}
        <span
          tabIndex={enabled ? -1 : 0}
          title={enabled ? undefined : action.reason}
          className="inline-flex"
        >
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={action.label}
            disabled={!enabled}
            onClick={(e) => {
              e.stopPropagation();
              action.run();
            }}
            className={cn(
              action.variant === "destructive" && "text-destructive",
              className,
            )}
          >
            {action.icon ?? <Play />}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}

/** One host action as a menu item; a disabled one says why under its label. */
export function ActionMenuItem({ action }: { action: HostAction }) {
  const enabled = action.enabled ?? true;
  return (
    <DropdownMenuItem
      aria-label={action.label}
      aria-description={!enabled ? action.reason : undefined}
      disabled={!enabled}
      variant={action.variant === "destructive" ? "destructive" : "default"}
      onSelect={action.run}
      className="items-start text-xs [&_svg]:mt-px [&_svg]:size-3.5"
    >
      {action.icon ?? <span className="size-3.5 shrink-0" />}
      <span className="flex min-w-0 flex-col">
        <span>{action.label}</span>
        {!enabled && action.reason && (
          <span className="text-[11px] text-muted-foreground">
            {action.reason}
          </span>
        )}
      </span>
    </DropdownMenuItem>
  );
}

/** A ⋯ button opening `leading` (e.g. Rename), then the host's `actions`
 *  after a separator. Draws nothing when both are empty. */
export function ActionsMenu({
  label,
  leading,
  actions,
  onOpenChange,
  className,
}: {
  /** The button's accessible name. */
  label: string;
  leading?: ReactNode;
  actions: readonly HostAction[];
  onOpenChange?: (open: boolean) => void;
  className?: string;
}) {
  // An entry that was picked may have moved focus (Rename's field, Remove's
  // confirmation): closing leaves it there rather than on the ⋯ button. Not
  // modal, so the menu doesn't trap that focus while it closes.
  const picked = useRef(false);
  if (!leading && actions.length === 0) return null;
  return (
    <DropdownMenu
      modal={false}
      onOpenChange={(open) => {
        if (open) picked.current = false;
        onOpenChange?.(open);
      }}
    >
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={label}
          title={label}
          onClick={(e) => e.stopPropagation()}
          className={className}
        >
          <Ellipsis />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        // Closes at once, like an editor's menus: reopened while it still
        // faded out, the click reopening it would also count as outside it.
        className="min-w-[11rem] max-w-72 data-[state=closed]:animate-none"
        // Keyboard selection clicks the entry too.
        onClickCapture={() => {
          picked.current = true;
        }}
        onCloseAutoFocus={(e) => {
          if (picked.current) e.preventDefault();
        }}
      >
        {leading}
        {leading && actions.length > 0 && <DropdownMenuSeparator />}
        {actions.map((action) => (
          <ActionMenuItem key={action.id} action={action} />
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
