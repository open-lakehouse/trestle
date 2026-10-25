// Small pieces of the workspace shell's chrome, shared by its views.

import {
  Button,
  Input,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@open-lakehouse/ui-kit";
import {
  AlertTriangle,
  ChevronRight,
  CircleCheck,
  CircleX,
  Loader2,
} from "lucide-react";
import { Fragment, type ReactNode, useState } from "react";
import type { MemberPlan } from "./useMemberPlans";

/** A sidebar view's title row, with its actions on the right. */
export function SidebarHeader({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex h-9 shrink-0 items-center justify-between px-3">
      <span className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </span>
      <div className="flex items-center gap-0.5">{children}</div>
    </div>
  );
}

/** A small icon-only button with a tooltip. */
export function IconButton({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label={label}
          disabled={disabled}
          onClick={onClick}
          className="h-6 w-6 [&_svg]:h-3.5 [&_svg]:w-3.5"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/** A member's state: planning, failed, clashing with another, or fine. */
export function EnvStatus({
  plan,
  clashing,
}: {
  plan: MemberPlan | undefined;
  clashing?: boolean;
}) {
  if (!plan) {
    return (
      <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
    );
  }
  if (plan.error) {
    return <CircleX className="h-3.5 w-3.5 shrink-0 text-destructive" />;
  }
  if (clashing) {
    return <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-status-ready" />;
  }
  return <CircleCheck className="h-3.5 w-3.5 shrink-0 text-success" />;
}

/** An inline name field: Enter submits (keeping it open on a refusal), Escape
 *  or leaving cancels. */
export function NameInput({
  initial,
  onSubmit,
  onDone,
}: {
  initial: string;
  onSubmit: (name: string) => string | undefined;
  onDone: () => void;
}) {
  const [value, setValue] = useState(initial);
  const [problem, setProblem] = useState<string>();
  return (
    <div>
      <Input
        autoFocus
        value={value}
        aria-label="Environment name"
        aria-invalid={!!problem}
        className="h-7 px-2 text-sm"
        onFocus={(e) => e.currentTarget.select()}
        onChange={(e) => {
          setValue(e.target.value);
          setProblem(undefined);
        }}
        onBlur={onDone}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
          if (e.key !== "Enter") return;
          const refused = onSubmit(value.trim());
          if (refused) setProblem(refused);
          else onDone();
        }}
      />
      {problem && <p className="mt-1 text-xs text-destructive">{problem}</p>}
    </div>
  );
}

/** Where an editor sits, `workspace › env › …`, for its toolbar. */
export function Breadcrumb({ parts }: { parts: readonly string[] }) {
  return (
    <nav
      aria-label="Breadcrumb"
      className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground"
    >
      {parts.map((part, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a path's parts are positional
        <Fragment key={i}>
          {i > 0 && <ChevronRight className="h-3 w-3 shrink-0" />}
          <span
            className={
              i === parts.length - 1 ? "truncate text-foreground" : "truncate"
            }
          >
            {part}
          </span>
        </Fragment>
      ))}
    </nav>
  );
}
