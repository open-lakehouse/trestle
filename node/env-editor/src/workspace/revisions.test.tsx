import { describe, expect, test } from "bun:test";
import { TooltipProvider } from "@open-lakehouse/ui-kit";
import { renderToStaticMarkup } from "react-dom/server";
import type { Selection } from "../types.js";
import {
  ContributionsProvider,
  type EditorContributions,
} from "./contributions.js";
import type { WorkspaceDoc } from "./model.js";
import { RevisionsView } from "./revisions.js";
import { createMemoryWorkspaceStore } from "./store.js";
import { WorkspaceProvider } from "./useWorkspace.js";

const selection = {
  modules: ["postgres"],
  capabilities: [],
  knob_overrides: {},
} as unknown as Selection;

const workspace: WorkspaceDoc = {
  name: "local",
  mesh: false,
  envs: [{ id: "dev", name: "dev", slot: 0, selection }],
};

const history = (contributions?: EditorContributions) =>
  renderToStaticMarkup(
    <TooltipProvider>
      <WorkspaceProvider
        store={createMemoryWorkspaceStore(workspace, { revisions: true })}
      >
        <ContributionsProvider contributions={contributions}>
          <RevisionsView
            workspace={workspace}
            env={workspace.envs[0]}
            catalog={undefined}
            onOpen={() => {}}
          />
        </ContributionsProvider>
      </WorkspaceProvider>
    </TooltipProvider>,
  );

describe("the history's commit", () => {
  test("a plain, compact Commit without the host's commit actions", () => {
    const out = history();
    expect(out).toMatch(/<button[^>]*h-6[^>]*>.*?Commit<\/button>/);
    expect(out).not.toContain("More ways to commit");
  });

  test("a split Commit with them", () => {
    const out = history({
      environment: {
        commitActions: () => [
          { id: "commit-start", label: "Commit and start", run: () => {} },
        ],
      },
    });
    expect(out).toContain('aria-label="More ways to commit"');
  });
});
