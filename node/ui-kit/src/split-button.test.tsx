import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { DropdownMenuItem } from "./dropdown-menu.js";
import { SplitButton } from "./split-button.js";

const menu = <DropdownMenuItem>Commit and start</DropdownMenuItem>;

describe("SplitButton", () => {
  test("a main button and a chevron that names its menu, the same size", () => {
    const out = renderToStaticMarkup(
      <SplitButton size="xs" menu={menu} menuLabel="More ways to commit">
        Commit
      </SplitButton>,
    );
    const buttons = out.match(/<button[^>]*>/g) ?? [];
    expect(buttons).toHaveLength(2);
    expect(buttons[1]).toContain('aria-label="More ways to commit"');
    for (const b of buttons) expect(b).toContain("h-6");
    // The menu is closed until the chevron opens it.
    expect(out).not.toContain("Commit and start");
  });

  test("disabled disables both halves; menuDisabled only the chevron", () => {
    const both = renderToStaticMarkup(
      <SplitButton disabled menu={menu}>
        Commit
      </SplitButton>,
    );
    expect(both.match(/disabled=""/g)).toHaveLength(2);
    const chevron = renderToStaticMarkup(
      <SplitButton menuDisabled menu={menu}>
        Commit
      </SplitButton>,
    );
    expect(chevron.match(/disabled=""/g)).toHaveLength(1);
  });
});
