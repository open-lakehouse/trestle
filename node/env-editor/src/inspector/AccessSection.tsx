import { Label, Switch } from "@open-lakehouse/ui-kit";
import type { GatewaySettings } from "../types";

const OPEN: GatewaySettings = { auth: false, policy: false };

/**
 * How the gateway guards access. Each switch brings in its provider by role
 * (Authelia for sign-in, OPA for policy); policy needs sign-in, so turning it
 * on turns sign-in on and turning sign-in off turns policy off.
 */
export function AccessSection({
  gateway = OPEN,
  onChange,
}: {
  /** The current settings; omitted means open. */
  gateway: GatewaySettings | undefined;
  onChange: (gateway: GatewaySettings) => void;
}) {
  const signIn = gateway.auth || gateway.policy;
  return (
    <div className="flex flex-col gap-3">
      <AccessSwitch
        id="gateway.auth"
        label="Require sign-in"
        help="Single-sign-on (Authelia) for API and UI routes. Object stores and databases are never gated."
        checked={signIn}
        onChange={(on) =>
          onChange(
            on ? { ...gateway, auth: true } : { auth: false, policy: false },
          )
        }
      />
      <AccessSwitch
        id="gateway.policy"
        label="Enforce policy"
        help="Authorize each signed-in request with a Rego policy (OPA). Turns on sign-in."
        checked={gateway.policy}
        onChange={(on) => onChange({ auth: on || gateway.auth, policy: on })}
      />
    </div>
  );
}

function AccessSwitch({
  id,
  label,
  help,
  checked,
  onChange,
}: {
  id: string;
  label: string;
  help: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <Label htmlFor={id} className="text-foreground">
          {label}
        </Label>
        <Switch id={id} checked={checked} onCheckedChange={onChange} />
      </div>
      <p className="text-[11px] text-muted-foreground">{help}</p>
    </div>
  );
}
