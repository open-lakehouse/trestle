import { Label, Switch } from "@open-lakehouse/ui-kit";
import type { GatewaySettings } from "../types";

/** An open gateway: no sign-in, no policy. */
export const OPEN: GatewaySettings = { auth: false, policy: false };

/** The settings for running without a gateway: sign-in and policy need it. */
export const DIRECT: GatewaySettings = {
  enabled: false,
  auth: false,
  policy: false,
};

/** Whether `gateway` runs a gateway (omitted means it does). */
export const gatewayEnabled = (gateway: GatewaySettings | undefined) =>
  gateway?.enabled !== false;

/**
 * Whether the environment runs a gateway, and how it guards access. Without
 * one, each service is published on its own host port. Each access switch
 * brings in its provider by role (Authelia for sign-in, OPA for policy);
 * policy needs sign-in, so turning it on turns sign-in on and turning sign-in
 * off turns policy off. Both need the gateway, so they show only while it runs.
 */
export function AccessSection({
  gateway = OPEN,
  onChange,
}: {
  /** The current settings; omitted means open. */
  gateway: GatewaySettings | undefined;
  onChange: (gateway: GatewaySettings) => void;
}) {
  const enabled = gatewayEnabled(gateway);
  return (
    <div className="flex flex-col gap-3">
      <AccessSwitch
        id="gateway.enabled"
        label="Run a gateway"
        help={
          enabled
            ? "One front door for every API and UI, on a single port."
            : "No gateway: each service is published on its own host port. Sign-in and policy need the gateway."
        }
        checked={enabled}
        onChange={(on) => onChange(on ? OPEN : DIRECT)}
      />
      {enabled && <AccessSwitches gateway={gateway} onChange={onChange} />}
    </div>
  );
}

/** The sign-in and policy switches of a running gateway. */
function AccessSwitches({
  gateway,
  onChange,
}: {
  gateway: GatewaySettings;
  onChange: (gateway: GatewaySettings) => void;
}) {
  const signIn = gateway.auth || gateway.policy;
  return (
    <>
      <AccessSwitch
        id="gateway.auth"
        label="Require sign-in"
        help="Single-sign-on (Authelia) for API and UI routes. Object stores and databases are never gated."
        checked={signIn}
        onChange={(on) =>
          onChange(
            on
              ? { auth: true, policy: gateway.policy }
              : { auth: false, policy: false },
          )
        }
      />
      <AccessSwitch
        id="gateway.policy"
        label="Enforce policy"
        help="Authorize each signed-in request with a Rego policy (OPA). Turns on sign-in."
        checked={gateway.policy}
        onChange={(on) =>
          onChange(
            on
              ? { auth: true, policy: true }
              : { auth: gateway.auth, policy: false },
          )
        }
      />
    </>
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
