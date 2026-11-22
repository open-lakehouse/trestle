import { ChevronDown } from "lucide-react";
import type * as React from "react";

import { Button, type ButtonProps } from "./button.js";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "./dropdown-menu.js";
import { cn } from "./utils.js";

export interface SplitButtonProps
  extends Omit<ButtonProps, "asChild" | "children"> {
  /** The main button's content. */
  children: React.ReactNode;
  /** The menu's items (`DropdownMenuItem`s), opened by the chevron. */
  menu: React.ReactNode;
  /** The chevron's accessible name. */
  menuLabel?: string;
  /** Disables only the chevron; `disabled` disables both halves. */
  menuDisabled?: boolean;
}

/** A button with a chevron that opens more ways to do the same thing, e.g.
 *  Commit with "Commit and start" in its menu. Both halves share the
 *  variant and size, and read as one control. */
function SplitButton({
  children,
  menu,
  menuLabel = "More options",
  menuDisabled,
  variant,
  size,
  className,
  disabled,
  ...props
}: SplitButtonProps) {
  return (
    <div className={cn("inline-flex items-stretch", className)}>
      <Button
        variant={variant}
        size={size}
        disabled={disabled}
        className="rounded-r-none"
        {...props}
      >
        {children}
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant={variant}
            size={size}
            disabled={disabled || menuDisabled}
            aria-label={menuLabel}
            className={cn(
              "rounded-l-none px-1",
              variant === "outline"
                ? "-ml-px"
                : "border-l border-l-primary-foreground/25",
            )}
          >
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-[10rem]">
          {menu}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

export { SplitButton };
