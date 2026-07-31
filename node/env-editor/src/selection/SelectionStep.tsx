import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@open-lakehouse/ui-kit";
import type { CatalogDto } from "../types";
import { CapabilityPicker } from "./CapabilityPicker";
import { TechnologyPicker } from "./TechnologyPicker";

export interface SelectionStepProps {
  catalog: CatalogDto;
  selectedModules: string[];
  selectedCapabilities: string[];
  overrides: Record<string, Record<string, string>>;
  onToggleModule: (moduleId: string) => void;
  onToggleCapability: (capability: string) => void;
  onSetKnob: (moduleId: string, key: string, value: string) => void;
}

/**
 * Define an environment by picking **Technologies** (specific modules) and/or
 * **Capabilities** (what you want, mapped to providers). The planner unions both.
 */
export function SelectionStep({
  catalog,
  selectedModules,
  selectedCapabilities,
  overrides,
  onToggleModule,
  onToggleCapability,
  onSetKnob,
}: SelectionStepProps) {
  return (
    <Tabs defaultValue="technologies" className="w-full">
      <TabsList>
        <TabsTrigger value="technologies">Technologies</TabsTrigger>
        <TabsTrigger value="capabilities">Capabilities</TabsTrigger>
      </TabsList>
      <TabsContent value="technologies">
        <TechnologyPicker
          catalog={catalog}
          selected={selectedModules}
          overrides={overrides}
          onToggle={onToggleModule}
          onSetKnob={onSetKnob}
        />
      </TabsContent>
      <TabsContent value="capabilities">
        <CapabilityPicker
          catalog={catalog}
          selected={selectedCapabilities}
          onToggle={onToggleCapability}
        />
      </TabsContent>
    </Tabs>
  );
}
