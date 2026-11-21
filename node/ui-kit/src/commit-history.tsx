import { Check, Copy, GitCommitHorizontal } from "lucide-react";
import { type ReactNode, useState } from "react";

import { cn } from "./utils.js";

/** Who made a commit. */
export interface CommitAuthor {
  name: string;
  /** Drawn in place of the initials, where the host has one. */
  avatarUrl?: string;
}

/** One commit (or revision, version…) in a `CommitHistory`. */
export interface CommitHistoryEntry {
  /** Stable identity: the React key and what `selectedId` matches. */
  id: string;
  /** The first line of the message, or a fallback such as "Revision #3". */
  title: ReactNode;
  /** The rest of the message, clamped to two lines. */
  description?: ReactNode;
  /** A short hash or digest, drawn in mono with a copy button. */
  ref?: string;
  /** The full hash, for the ref's tooltip and what the copy button copies. */
  refTitle?: string;
  author?: CommitAuthor;
  /** When it was made (a `Date` or an ISO 8601 string). Entries without one
   *  are grouped under "Undated". */
  timestamp?: string | Date;
  /** Small tags next to the ref, e.g. a number or "HEAD". */
  badges?: ReactNode;
}

/** The entries made on one calendar day, or the undated ones. */
export interface CommitDayGroup {
  /** `YYYY-MM-DD` in local time, or `"undated"`. */
  key: string;
  /** Midnight of that day; absent for the undated group. */
  day?: Date;
  entries: CommitHistoryEntry[];
}

function toDate(value: string | Date | undefined): Date | undefined {
  if (value === undefined) return undefined;
  const date = value instanceof Date ? value : new Date(value);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

const pad = (n: number) => String(n).padStart(2, "0");

/**
 * Group `entries` by local calendar day, keeping their order: groups appear in
 * the order of their first entry (newest first when the entries are), and
 * entries without a (valid) timestamp come last, under `"undated"`.
 */
export function groupByDay(
  entries: readonly CommitHistoryEntry[],
): CommitDayGroup[] {
  const groups = new Map<string, CommitDayGroup>();
  const undated: CommitHistoryEntry[] = [];
  for (const entry of entries) {
    const date = toDate(entry.timestamp);
    if (!date) {
      undated.push(entry);
      continue;
    }
    const key = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
    let group = groups.get(key);
    if (!group) {
      group = {
        key,
        day: new Date(date.getFullYear(), date.getMonth(), date.getDate()),
        entries: [],
      };
      groups.set(key, group);
    }
    group.entries.push(entry);
  }
  const out = [...groups.values()];
  if (undated.length > 0) out.push({ key: "undated", entries: undated });
  return out;
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["minute", 60],
  ["hour", 60 * 60],
  ["day", 24 * 60 * 60],
  ["week", 7 * 24 * 60 * 60],
  ["month", 30 * 24 * 60 * 60],
  ["year", 365 * 24 * 60 * 60],
];

/**
 * How long ago `date` was, relative to `now`: "just now" under a minute, then
 * the largest whole unit ("5 minutes ago", "yesterday", "3 weeks ago").
 * A `date` after `now` reads "just now" too (clock skew between hosts).
 */
export function formatRelativeTime(
  date: Date,
  now: Date = new Date(),
  locale?: string,
): string {
  const seconds = Math.round((now.getTime() - date.getTime()) / 1000);
  if (seconds < 60) return "just now";
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  let unit = UNITS[0];
  for (const candidate of UNITS) {
    if (seconds >= candidate[1]) unit = candidate;
  }
  return rtf.format(-Math.floor(seconds / unit[1]), unit[0]);
}

function defaultDayLabel(day: Date): string {
  return day.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

export interface CommitHistoryProps {
  /** Newest first. */
  entries: readonly CommitHistoryEntry[];
  /** Makes each entry's title a button. */
  onSelect?: (entry: CommitHistoryEntry) => void;
  /** The highlighted entry. */
  selectedId?: string;
  /** Per-entry actions (e.g. compare), shown on hover and focus. */
  renderActions?: (entry: CommitHistoryEntry) => ReactNode;
  /** Group the entries under a heading per day. Defaults to true. */
  groupByDay?: boolean;
  /** Narrow layout for sidebars: the ref and badges move under the title. */
  compact?: boolean;
  /** A day's heading. Defaults to the date, e.g. "Nov 21, 2026". */
  dayLabel?: (day: Date) => ReactNode;
  /** Drawn when there are no entries. */
  emptyState?: ReactNode;
  /** The reference time for relative timestamps. Defaults to now. */
  now?: Date;
  "aria-label"?: string;
  className?: string;
}

/**
 * A commit history in the style of GitHub's commits page: the entries grouped
 * by day along a timeline, each with its title, author, relative time, badges
 * and a copyable ref. Generic over what is being versioned: the host maps its
 * own records (revisions, commits, table versions) to `CommitHistoryEntry`.
 */
export function CommitHistory({
  entries,
  onSelect,
  selectedId,
  renderActions,
  groupByDay: grouped = true,
  compact = false,
  dayLabel = defaultDayLabel,
  emptyState,
  now,
  "aria-label": ariaLabel,
  className,
}: CommitHistoryProps) {
  if (entries.length === 0) {
    return (
      <div className={cn("text-xs text-muted-foreground", className)}>
        {emptyState ?? "No commits yet."}
      </div>
    );
  }
  const groups = grouped
    ? groupByDay(entries)
    : [{ key: "all", entries: [...entries] } as CommitDayGroup];
  const row = (entry: CommitHistoryEntry) => (
    <CommitRow
      key={entry.id}
      entry={entry}
      selected={entry.id === selectedId}
      onSelect={onSelect}
      actions={renderActions?.(entry)}
      compact={compact}
      now={now}
    />
  );
  return (
    <section aria-label={ariaLabel} className={cn("text-sm", className)}>
      {groups.map((group) => (
        <div key={group.key}>
          {grouped && (
            <h3 className="flex items-center gap-1.5 py-1 text-xs text-muted-foreground">
              <GitCommitHorizontal
                aria-hidden
                className="h-3.5 w-3.5 shrink-0"
              />
              {group.day ? dayLabel(group.day) : "Undated"}
            </h3>
          )}
          <div className={cn(grouped && "ml-[7px] border-l pb-3 pl-3")}>
            <ol className="divide-y overflow-hidden rounded-md border bg-card text-card-foreground">
              {group.entries.map(row)}
            </ol>
          </div>
        </div>
      ))}
    </section>
  );
}

function CommitRow({
  entry,
  selected,
  onSelect,
  actions,
  compact,
  now,
}: {
  entry: CommitHistoryEntry;
  selected: boolean;
  onSelect?: (entry: CommitHistoryEntry) => void;
  actions: ReactNode;
  compact: boolean;
  now?: Date;
}) {
  const date = toDate(entry.timestamp);
  const titleClass = "block w-full truncate text-left font-medium";
  const tags = (entry.badges || entry.ref) && (
    <span className="flex shrink-0 items-center gap-1">
      {entry.badges}
      {entry.ref && <RefChip value={entry.ref} full={entry.refTitle} />}
    </span>
  );
  return (
    <li
      className={cn(
        "group flex items-start gap-2",
        compact ? "px-2 py-1.5" : "px-3 py-2",
        selected && "bg-accent text-accent-foreground",
      )}
    >
      <div className="min-w-0 flex-1">
        {onSelect ? (
          <button
            type="button"
            className={cn(
              titleClass,
              "rounded-sm hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            )}
            aria-current={selected || undefined}
            onClick={() => onSelect(entry)}
          >
            {entry.title}
          </button>
        ) : (
          <span className={titleClass}>{entry.title}</span>
        )}
        {entry.description && (
          <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
            {entry.description}
          </p>
        )}
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
          {entry.author && (
            <>
              <Avatar author={entry.author} />
              <span className="truncate font-medium text-foreground">
                {entry.author.name}
              </span>
            </>
          )}
          {date && (
            <time dateTime={date.toISOString()} title={date.toLocaleString()}>
              {entry.author && !compact ? "committed " : ""}
              {formatRelativeTime(date, now)}
            </time>
          )}
          {compact && tags}
        </div>
      </div>
      {!compact && tags}
      {actions && (
        <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
          {actions}
        </span>
      )}
    </li>
  );
}

function Avatar({ author }: { author: CommitAuthor }) {
  if (author.avatarUrl) {
    return (
      <img
        src={author.avatarUrl}
        alt=""
        className="h-4 w-4 shrink-0 rounded-full"
      />
    );
  }
  const initials = author.name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");
  return (
    <span
      aria-hidden
      className="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-muted text-[8px] font-semibold text-muted-foreground"
    >
      {initials}
    </span>
  );
}

/** A short ref in mono, copying the full one on a click. */
function RefChip({ value, full }: { value: string; full?: string }) {
  const [copied, setCopied] = useState(false);
  const text = full ?? value;
  const copy = () => {
    void navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <span
      className="inline-flex items-center rounded border bg-background font-mono text-[11px] text-muted-foreground"
      title={text}
    >
      <span className="px-1">{value}</span>
      <button
        type="button"
        aria-label={`Copy ${text}`}
        onClick={copy}
        className="border-l px-0.5 py-0.5 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&_svg]:h-3 [&_svg]:w-3"
      >
        {copied ? <Check /> : <Copy />}
      </button>
    </span>
  );
}
