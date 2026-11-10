// A line diff for the default diff viewer: the longest common subsequence of
// two texts' lines, as kept, added and removed lines, with long runs of
// unchanged lines folded away. Rendered files are small (config, compose), so
// the quadratic table is fine.

/** One line of a diff: kept (` `), added (`+`) or removed (`-`). */
export interface DiffLine {
  op: " " | "+" | "-";
  text: string;
}

/** Unchanged lines folded out of a diff. */
export interface DiffFold {
  op: "fold";
  /** How many lines it hides. */
  count: number;
}

const linesOf = (text: string | undefined): string[] => {
  if (!text) return [];
  const lines = text.split("\n");
  // A trailing newline doesn't start another line.
  if (lines.at(-1) === "") lines.pop();
  return lines;
};

/** Every line of `before` and `after`, in order, marked kept, added or
 *  removed. An absent text has no lines. */
export function lineDiff(before?: string, after?: string): DiffLine[] {
  const a = linesOf(before);
  const b = linesOf(after);
  // lcs[i][j]: the longest common subsequence of a[i..] and b[j..].
  const lcs = Array.from({ length: a.length + 1 }, () =>
    new Array<number>(b.length + 1).fill(0),
  );
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] =
        a[i] === b[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ op: " ", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ op: "-", text: a[i++] });
    } else {
      out.push({ op: "+", text: b[j++] });
    }
  }
  while (i < a.length) out.push({ op: "-", text: a[i++] });
  while (j < b.length) out.push({ op: "+", text: b[j++] });
  return out;
}

/** `lines` with unchanged runs more than `context` lines from any change
 *  folded away. */
export function foldContext(
  lines: readonly DiffLine[],
  context = 3,
): (DiffLine | DiffFold)[] {
  const changed = lines.map((l) => l.op !== " ");
  const near = lines.map((_, i) => {
    for (let k = Math.max(0, i - context); k <= i + context; k++) {
      if (changed[k]) return true;
    }
    return false;
  });
  const out: (DiffLine | DiffFold)[] = [];
  for (let i = 0; i < lines.length; i++) {
    if (near[i]) {
      out.push(lines[i]);
      continue;
    }
    const last = out.at(-1);
    if (last?.op === "fold") last.count++;
    else out.push({ op: "fold", count: 1 });
  }
  return out;
}

/** Whether two texts differ, line by line. */
export function textChanged(before?: string, after?: string): boolean {
  return (before ?? "") !== (after ?? "");
}
