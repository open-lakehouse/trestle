import { describe, expect, test } from "bun:test";
import { renderToString } from "react-dom/server";
import { InlineConfirm } from "./chrome.js";

describe("InlineConfirm", () => {
  test("asks in the page, with the action and a way out", () => {
    const html = renderToString(
      <InlineConfirm
        message="Remove dev from the workspace?"
        action="Remove"
        onConfirm={() => {}}
        onCancel={() => {}}
      />,
    );
    expect(html).toContain('role="alertdialog"');
    expect(html).toContain("Remove dev from the workspace?");
    expect(html).toContain(">Remove</button>");
    expect(html).toContain(">Cancel</button>");
  });
});
