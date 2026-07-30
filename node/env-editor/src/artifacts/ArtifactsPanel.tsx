import {
  Button,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@open-lakehouse/ui-kit";
import { Check, Copy } from "lucide-react";
import { useState } from "react";
import type { PlanResult } from "../types";

export interface ArtifactsPanelProps {
  plan: PlanResult;
}

interface ArtifactTab {
  value: string;
  label: string;
  content: string;
  language: string;
}

/** A code block with a copy-to-clipboard button. */
function CodeBlock({ content }: { content: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard.writeText(content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <div className="relative">
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="absolute right-2 top-2 h-7 w-7"
        onClick={copy}
        aria-label="Copy to clipboard"
      >
        {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
      </Button>
      <pre className="max-h-[420px] overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-foreground">
        <code>{content}</code>
      </pre>
    </div>
  );
}

/**
 * The rendered environment artifacts, one per tab: the top-level compose file,
 * the Envoy gateway bootstrap, the `.env` overlay, and the Markdown layout report.
 */
export function ArtifactsPanel({ plan }: ArtifactsPanelProps) {
  const tabs: ArtifactTab[] = [
    {
      value: "compose",
      label: "compose.yaml",
      content: plan.artifacts.compose,
      language: "yaml",
    },
    {
      value: "envoy",
      label: "envoy.yaml",
      content: plan.artifacts.envoy,
      language: "yaml",
    },
    {
      value: "env",
      label: ".env",
      content: plan.artifacts.env,
      language: "dotenv",
    },
    {
      value: "layout",
      label: "LAYOUT.md",
      content: plan.layout_report,
      language: "markdown",
    },
  ];

  return (
    <Tabs defaultValue="compose" className="w-full">
      <TabsList>
        {tabs.map((t) => (
          <TabsTrigger key={t.value} value={t.value}>
            {t.label}
          </TabsTrigger>
        ))}
      </TabsList>
      {tabs.map((t) => (
        <TabsContent key={t.value} value={t.value}>
          <CodeBlock content={t.content} />
        </TabsContent>
      ))}
    </Tabs>
  );
}
