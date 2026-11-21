import {
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@open-lakehouse/ui-kit";
import {
  FEATURE_LABEL,
  featureAvailable,
  featureProvider,
  gatewayEnabled,
  PLATFORM_FEATURES,
  providersOf,
} from "../selection/platform.js";
import type {
  CatalogDto,
  FeatureChoice,
  PlatformFeature,
  Selection,
} from "../types.js";

/**
 * Whether the environment runs a gateway, and which platform features it turns
 * on: identity, policy and observability, each with its provider. Every module
 * that can integrate with a feature does, unless its inspector turns that off.
 * Identity and policy need the gateway; policy needs identity.
 */
export function PlatformSection({
  catalog,
  selection,
  onSetGateway,
  onSetFeature,
}: {
  catalog: CatalogDto | undefined;
  selection: Selection;
  onSetGateway: (on: boolean) => void;
  onSetFeature: (
    feature: PlatformFeature,
    choice: FeatureChoice | undefined,
  ) => void;
}) {
  const enabled = gatewayEnabled(selection);
  return (
    <div className="flex flex-col gap-3">
      <PlatformSwitch
        id="gateway.enabled"
        label="Run a gateway"
        help={
          enabled
            ? "One front door for every API and UI, on a single port."
            : "No gateway: each service is published on its own host port. Identity and policy need the gateway."
        }
        checked={enabled}
        onChange={onSetGateway}
      />
      {PLATFORM_FEATURES.filter((f) => featureAvailable(selection, f)).map(
        (feature) => (
          <FeatureControl
            key={feature}
            feature={feature}
            catalog={catalog}
            selection={selection}
            onChange={(choice) => onSetFeature(feature, choice)}
          />
        ),
      )}
    </div>
  );
}

/** One feature's switch, and its provider when the catalog has several. */
function FeatureControl({
  feature,
  catalog,
  selection,
  onChange,
}: {
  feature: PlatformFeature;
  catalog: CatalogDto | undefined;
  selection: Selection;
  onChange: (choice: FeatureChoice | undefined) => void;
}) {
  const providers = catalog ? providersOf(catalog, feature) : [];
  const on = selection.platform?.[feature] !== undefined;
  const provider = catalog
    ? featureProvider(catalog, selection, feature)
    : undefined;
  const { title, help } = FEATURE_LABEL[feature];
  const providerName = (id: string | undefined) =>
    providers.find((p) => p.id === id)?.display_name ?? id;
  return (
    <div className="flex flex-col gap-1">
      <PlatformSwitch
        id={`platform.${feature}`}
        label={title}
        help={
          on && providers.length <= 1 && provider
            ? `${help} Provided by ${providerName(provider)}.`
            : help
        }
        checked={on}
        disabled={catalog !== undefined && providers.length === 0}
        onChange={(next) => onChange(next ? true : undefined)}
      />
      {on && providers.length > 1 && (
        <Select
          value={provider}
          onValueChange={(id) => onChange(id === providers[0]?.id ? true : id)}
        >
          <SelectTrigger
            className="h-8 text-xs"
            aria-label={`${title} provider`}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {providers.map((p) => (
              <SelectItem key={p.id} value={p.id} className="text-xs">
                {p.display_name ?? p.id}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
    </div>
  );
}

function PlatformSwitch({
  id,
  label,
  help,
  checked,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  help: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <Label htmlFor={id} className="text-foreground">
          {label}
        </Label>
        <Switch
          id={id}
          checked={checked}
          disabled={disabled}
          onCheckedChange={onChange}
        />
      </div>
      <p className="text-[11px] text-muted-foreground">{help}</p>
    </div>
  );
}
