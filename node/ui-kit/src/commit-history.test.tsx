import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import {
  CommitHistory,
  type CommitHistoryEntry,
  formatRelativeTime,
  groupByDay,
} from "./commit-history.js";

const NOW = new Date(2026, 10, 21, 15, 0);
const ago = (seconds: number) => new Date(NOW.getTime() - seconds * 1000);

describe("groupByDay", () => {
  test("groups by local day in entry order, undated last", () => {
    const entries: CommitHistoryEntry[] = [
      { id: "a", title: "a", timestamp: new Date(2026, 10, 21, 14) },
      { id: "u", title: "u" },
      { id: "b", title: "b", timestamp: new Date(2026, 10, 21, 1) },
      { id: "c", title: "c", timestamp: new Date(2026, 10, 19, 23, 59) },
      { id: "bad", title: "bad", timestamp: "not a date" },
    ];
    const groups = groupByDay(entries);
    expect(groups.map((g) => g.key)).toEqual([
      "2026-11-21",
      "2026-11-19",
      "undated",
    ]);
    expect(groups[0].entries.map((e) => e.id)).toEqual(["a", "b"]);
    expect(groups[0].day).toEqual(new Date(2026, 10, 21));
    expect(groups[2].entries.map((e) => e.id)).toEqual(["u", "bad"]);
    expect(groups[2].day).toBeUndefined();
  });

  test("accepts ISO strings", () => {
    const iso = new Date(2026, 0, 2, 12).toISOString();
    expect(groupByDay([{ id: "x", title: "x", timestamp: iso }])[0].key).toBe(
      "2026-01-02",
    );
  });

  test("nothing in, nothing out", () => {
    expect(groupByDay([])).toEqual([]);
  });
});

describe("formatRelativeTime", () => {
  const rel = (seconds: number) => formatRelativeTime(ago(seconds), NOW, "en");

  test("under a minute, and the future, read just now", () => {
    expect(rel(0)).toBe("just now");
    expect(rel(59)).toBe("just now");
    expect(rel(-3600)).toBe("just now");
  });

  test("picks the largest whole unit", () => {
    expect(rel(60)).toBe("1 minute ago");
    expect(rel(59 * 60)).toBe("59 minutes ago");
    expect(rel(60 * 60)).toBe("1 hour ago");
    expect(rel(5 * 3600)).toBe("5 hours ago");
    expect(rel(24 * 3600)).toBe("yesterday");
    expect(rel(3 * 24 * 3600)).toBe("3 days ago");
    expect(rel(7 * 24 * 3600)).toBe("last week");
    expect(rel(15 * 24 * 3600)).toBe("2 weeks ago");
    expect(rel(45 * 24 * 3600)).toBe("last month");
    expect(rel(800 * 24 * 3600)).toBe("2 years ago");
  });
});

describe("CommitHistory", () => {
  const entries: CommitHistoryEntry[] = [
    {
      id: "2",
      title: "Add lineage",
      ref: "9f3c2a1b",
      refTitle: "9f3c2a1b7e4d",
      author: { name: "Ada Lovelace" },
      timestamp: ago(3600),
      badges: <span>HEAD</span>,
    },
    { id: "1", title: "Initial", timestamp: ago(3 * 24 * 3600) },
  ];

  test("renders day groups, rows, refs and actions", () => {
    const html = renderToStaticMarkup(
      <CommitHistory
        entries={entries}
        now={NOW}
        aria-label="History"
        dayLabel={(d) => `day ${d.getDate()}`}
        onSelect={() => {}}
        selectedId="2"
        renderActions={(e) => <button type="button">compare {e.id}</button>}
      />,
    );
    expect(html).toContain('aria-label="History"');
    expect(html).toContain("day 21");
    expect(html).toContain("day 18");
    expect(html.indexOf("day 21")).toBeLessThan(html.indexOf("day 18"));
    expect(html).toContain("Add lineage");
    expect(html).toContain('aria-current="true"');
    expect(html).toContain("committed 1 hour ago");
    expect(html).toContain("AL");
    expect(html).toContain("9f3c2a1b");
    expect(html).toContain('aria-label="Copy 9f3c2a1b7e4d"');
    expect(html).toContain("HEAD");
    expect(html).toContain("compare 1");
  });

  test("titles are plain text without onSelect", () => {
    const html = renderToStaticMarkup(
      <CommitHistory entries={entries} now={NOW} />,
    );
    expect(html).not.toContain("aria-current");
    expect(html).not.toContain("hover:underline");
  });

  test("ungrouped drops the headings", () => {
    const html = renderToStaticMarkup(
      <CommitHistory entries={entries} now={NOW} groupByDay={false} />,
    );
    expect(html).not.toContain("<h3");
  });

  test("shows the empty state", () => {
    const html = renderToStaticMarkup(
      <CommitHistory entries={[]} emptyState="No revisions yet." />,
    );
    expect(html).toContain("No revisions yet.");
  });
});
