import { describe, expect, test } from "bun:test";
import { renderToString } from "react-dom/server";
import {
  createLocalStorageWorkspaceStore,
  createMemoryWorkspaceStore,
  type WorkspaceStore,
} from "./store.js";
import { useWorkspace, WorkspaceProvider } from "./useWorkspace.js";

function Probe() {
  const { revisions } = useWorkspace();
  return <span>{revisions ? "revisions" : "drafts-only"}</span>;
}

const render = (store: WorkspaceStore) =>
  renderToString(
    <WorkspaceProvider store={store}>
      <Probe />
    </WorkspaceProvider>,
  );

describe("useWorkspaceState revisions", () => {
  test("stores without revisions offer none, so the shell hides them", () => {
    const storage = { getItem: () => null, setItem: () => {} };
    expect(render(createLocalStorageWorkspaceStore({ storage }))).toContain(
      "drafts-only",
    );
    expect(render(createMemoryWorkspaceStore())).toContain("drafts-only");
  });

  test("a store with revisions offers them", () => {
    expect(
      render(createMemoryWorkspaceStore(null, { revisions: true })),
    ).toContain("revisions");
  });
});
