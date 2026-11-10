import { describe, expect, test } from "bun:test";
import { foldContext, lineDiff } from "./lineDiff.js";

const ops = (before?: string, after?: string) =>
  lineDiff(before, after).map((l) => `${l.op}${l.text}`);

describe("lineDiff", () => {
  test("identical texts keep every line", () => {
    expect(ops("a\nb\n", "a\nb\n")).toEqual([" a", " b"]);
  });

  test("an added file is all additions, a removed one all removals", () => {
    expect(ops(undefined, "a\nb")).toEqual(["+a", "+b"]);
    expect(ops("a\nb", undefined)).toEqual(["-a", "-b"]);
  });

  test("a changed line is a removal then an addition, in place", () => {
    expect(ops("a\nb\nc", "a\nB\nc")).toEqual([" a", "-b", "+B", " c"]);
  });

  test("insertions and deletions around kept lines", () => {
    expect(ops("a\nc", "a\nb\nc\nd")).toEqual([" a", "+b", " c", "+d"]);
    expect(ops("x\na\ny", "a")).toEqual(["-x", " a", "-y"]);
  });
});

describe("foldContext", () => {
  test("folds unchanged runs away from changes, keeping context", () => {
    const before = Array.from({ length: 20 }, (_, i) => `l${i}`).join("\n");
    const after = before.replace("l10", "L10");
    const folded = foldContext(lineDiff(before, after), 2);
    expect(folded[0]).toEqual({ op: "fold", count: 8 });
    expect(folded.slice(1, 7).map((l) => l.op)).toEqual([
      " ",
      " ",
      "-",
      "+",
      " ",
      " ",
    ]);
    expect(folded.at(-1)).toEqual({ op: "fold", count: 7 });
  });

  test("an unchanged diff folds into one run", () => {
    expect(foldContext(lineDiff("a\nb", "a\nb"))).toEqual([
      { op: "fold", count: 2 },
    ]);
  });
});
