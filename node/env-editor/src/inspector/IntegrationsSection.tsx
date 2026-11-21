import { Label, Switch } from "@open-lakehouse/ui-kit";
import { FEATURE_LABEL, integrationsOf } from "../selection/platform.js";
import type { Integration, PlatformFeature } from "../types.js";

/**
 * How the inspected module integrates with each selected platform feature: on
 * by default, with a switch to opt it out, or why it can't. Only features the
 * selection turns on and the module can integrate with are listed.
 */
export function IntegrationsSection({
  moduleId,
  integrations,
  providerName,
  onChange,
}: {
  moduleId: string;
  /** The plan's integrations (all modules'). */
  integrations: Integration[] | undefined;
  /** A provider's display name. */
  providerName: (id: string) => string;
  onChange: (feature: PlatformFeature, on: boolean) => void;
}) {
  const rows = integrationsOf(integrations, moduleId);
  if (rows.length === 0) return null;
  return (
    <div className="flex flex-col gap-3">
      {rows.map((integration) => {
        const id = `integration.${moduleId}.${integration.feature}`;
        const incompatible = integration.state === "incompatible";
        return (
          <div key={integration.feature} className="flex flex-col gap-1">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor={id} className="text-foreground">
                {FEATURE_LABEL[integration.feature].title}
              </Label>
              <Switch
                id={id}
                checked={integration.state === "active"}
                disabled={incompatible}
                onCheckedChange={(on) => onChange(integration.feature, on)}
              />
            </div>
            <p className="text-[11px] text-muted-foreground">
              {describe(integration, providerName(integration.provider))}
            </p>
          </div>
        );
      })}
    </div>
  );
}

function describe(integration: Integration, provider: string): string {
  switch (integration.state) {
    case "active":
      return integration.protocol === "gateway-headers"
        ? `Takes the signed-in user from the gateway (${provider}).`
        : `With ${provider}, over ${integration.protocol}.`;
    case "disabled":
      return `Off for this component; ${provider} is still on for the rest.`;
    case "incompatible":
      return `Not compatible with ${provider}: ${integration.reason}.`;
  }
}
