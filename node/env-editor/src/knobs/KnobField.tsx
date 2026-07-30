import {
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@open-lakehouse/ui-kit";
import type { Knob } from "../types";

export interface KnobFieldProps {
  /** Stable field id (e.g. `${moduleId}.${knob.key}`). */
  fieldId: string;
  knob: Knob;
  /** The current value (override or default), always a string; `undefined` if unset. */
  value: string | undefined;
  /** Emit a new value (as a string, matching the Rust `knob_overrides` shape). */
  onChange: (value: string) => void;
}

/**
 * One knob control, chosen by `KnobKind`: `Switch` for Bool, `Select` for Enum,
 * a numeric `Input` for Integer/Port, a text `Input` for String. Values are
 * always strings to match `Selection.knob_overrides`.
 */
export function KnobField({ fieldId, knob, value, onChange }: KnobFieldProps) {
  const effective = value ?? knob.default ?? "";
  const title = knob.title ?? knob.key;

  const control = () => {
    switch (knob.kind.kind) {
      case "bool":
        return (
          <Switch
            id={fieldId}
            checked={effective === "true"}
            onCheckedChange={(checked) => onChange(checked ? "true" : "false")}
          />
        );
      case "enum":
        return (
          <Select value={effective} onValueChange={onChange}>
            <SelectTrigger id={fieldId}>
              <SelectValue placeholder="Select…" />
            </SelectTrigger>
            <SelectContent>
              {knob.kind.options.map((opt) => (
                <SelectItem key={opt} value={opt}>
                  {opt}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        );
      case "integer":
        return (
          <Input
            id={fieldId}
            type="number"
            min={knob.kind.min ?? undefined}
            max={knob.kind.max ?? undefined}
            value={effective}
            onChange={(e) => onChange(e.target.value)}
          />
        );
      case "port":
        return (
          <Input
            id={fieldId}
            type="number"
            min={1}
            max={65535}
            value={effective}
            onChange={(e) => onChange(e.target.value)}
          />
        );
      default:
        return (
          <Input
            id={fieldId}
            type="text"
            value={effective}
            onChange={(e) => onChange(e.target.value)}
          />
        );
    }
  };

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <Label htmlFor={fieldId} className="text-foreground">
          {title}
          {knob.required && <span className="ml-1 text-destructive">*</span>}
        </Label>
        {knob.kind.kind === "bool" && control()}
      </div>
      {knob.kind.kind !== "bool" && control()}
      {knob.help && (
        <p className="text-[11px] text-muted-foreground">{knob.help}</p>
      )}
    </div>
  );
}
