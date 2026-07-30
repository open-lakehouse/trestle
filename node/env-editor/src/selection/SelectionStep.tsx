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
  onToggleModule: (moduleId: string) => void;
  onToggleCapability: (capability: string) => void;
}

/**
 * Define an environment by picking **Technologies** (specific modules) and/or
 * **Capabilities** (what you want, mapped to providers). The planner unions both.
 */
export function SelectionStep({
  catalog,
  selectedModules,
  selectedCapabilities,
  onToggleModule,
  onToggleCapability,
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
          onToggle={onToggleModule}
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
