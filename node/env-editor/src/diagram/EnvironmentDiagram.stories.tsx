import { FIXTURE_PLAN } from "@open-lakehouse/stack-wasm/fixtures";
import type { Meta, StoryObj } from "@storybook/react";
import { type ReactNode, useState } from "react";
import { createPortal } from "react-dom";
import { useDiagramPortal } from "../graph/nodeSlots";
import { EnvironmentDiagram } from "./EnvironmentDiagram";
import { createGraphSnapshot } from "./snapshot";

// The read-only embed, drawn from a snapshot of the fixture plan, set in a
// prose-width column the way a blog post would host it.
const snapshot = createGraphSnapshot(FIXTURE_PLAN.graph, {
  title: "A local lakehouse",
});

const meta: Meta<typeof EnvironmentDiagram> = {
  title: "EnvironmentDiagram",
  component: EnvironmentDiagram,
  args: { snapshot },
  decorators: [
    (Story) => (
      <div style={{ maxWidth: 860, margin: "2rem auto" }}>
        <Story />
      </div>
    ),
  ],
};

export default meta;

type Story = StoryObj<typeof EnvironmentDiagram>;

export const Light: Story = { args: { colorMode: "light" } };

export const Dark: Story = { args: { colorMode: "dark" } };

export const FixedHeight: Story = { args: { height: 300, legend: false } };

export const UnsupportedSnapshot: Story = {
  args: { snapshot: { format: "trestle-graph", version: 99 } },
};

// A host linking each node's name and role to its own pages, with a hover card
// portaled where the diagram says (the body inline, inside the expand dialog).
function HoverLink({
  href,
  card,
  children,
}: {
  href: string;
  card: string;
  children: ReactNode;
}) {
  const portal = useDiagramPortal();
  const [at, setAt] = useState<DOMRect>();
  return (
    <>
      <a
        href={href}
        style={{ textDecoration: "underline dotted" }}
        onMouseEnter={(e) => setAt(e.currentTarget.getBoundingClientRect())}
        onMouseLeave={() => setAt(undefined)}
      >
        {children}
      </a>
      {at &&
        createPortal(
          <div
            // Host styles: outside the diagram's scope, its utilities don't apply.
            style={{
              position: "fixed",
              left: at.left,
              top: at.bottom + 6,
              zIndex: 50,
              padding: 8,
              font: "12px sans-serif",
              background: "white",
              border: "1px solid #ccc",
              borderRadius: 6,
            }}
          >
            {card}
          </div>,
          portal ?? document.body,
        )}
    </>
  );
}

export const WithSlots: Story = {
  args: {
    slots: {
      name: (node, fallback) =>
        node.model ? (
          <HoverLink
            href={`#${node.model}`}
            card={`Implementation: ${node.model}`}
          >
            {fallback}
          </HoverLink>
        ) : undefined,
      role: (node, fallback) =>
        node.role ? (
          <HoverLink href={`#${node.role}`} card={`Role: ${node.role}`}>
            {fallback}
          </HoverLink>
        ) : undefined,
    },
  },
};

// A host counting what each node manages in a trailing chip.
export const WithAside: Story = {
  args: {
    slots: {
      aside: (node) =>
        node.role === "data_catalog" ? (
          <HoverLink
            href={`#${node.model}-assets`}
            card="Tables, volumes, functions"
          >
            3
          </HoverLink>
        ) : undefined,
    },
  },
};

// A snapshot with its file manifest, the host reading contents from the
// render shipped next to it (here the fixture plan's files, after a delay).
const contents = new Map(FIXTURE_PLAN.files.map((f) => [f.path, f.contents]));
const readFixtureFile = (path: string) =>
  new Promise<string>((resolve, reject) =>
    setTimeout(() => {
      const found = contents.get(path);
      if (found === undefined) reject(new Error("not found"));
      else resolve(found);
    }, 300),
  );

export const WithFiles: Story = {
  args: {
    snapshot: createGraphSnapshot(FIXTURE_PLAN.graph, {
      title: "A local lakehouse",
      files: FIXTURE_PLAN.files.map((f) => f.path),
    }),
    readFile: readFixtureFile,
  },
};

// A host whose render is unreachable: the dialog says so per file.
export const WithUnreadableFiles: Story = {
  args: {
    ...WithFiles.args,
    readFile: () => Promise.reject(new Error("404 Not Found")),
  },
};
