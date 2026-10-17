import { cn } from "@open-lakehouse/ui-kit";
import { AlertTriangle, Check, Copy, Loader2 } from "lucide-react";
import {
  type CSSProperties,
  type ReactNode,
  useEffect,
  useMemo,
  useState,
} from "react";
import type { FileSource } from "./fileSource";
import {
  type CodeLine,
  highlightLines,
  languageFor,
  languageLabel,
  plainLines,
} from "./highlight";

export interface CodeViewerProps {
  path: string;
  read: FileSource["read"];
  /**
   * `"full"` heads the file with its name over its path (the Files tab);
   * `"compact"` with a single path line, for a viewer under file tabs that
   * already name it.
   */
  header?: "full" | "compact";
  className?: string;
}

type Contents =
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "ready"; text: string };

/**
 * A read-only, syntax-highlighted view of one rendered file, with line
 * numbers and a copy button. The contents come from `read`, synchronously
 * (the editor's plan) or not (an embed fetching from its companion render).
 */
export function CodeViewer({
  path,
  read,
  header = "full",
  className,
}: CodeViewerProps) {
  const contents = useContents(path, read);
  const lang = languageFor(path);

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
        <div className="min-w-0">
          {header === "full" && (
            <div className="truncate text-sm font-medium text-card-foreground">
              {path.split("/").pop()}
            </div>
          )}
          <div className="truncate font-mono text-[11px] text-muted-foreground">
            {path}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <span className="rounded border border-border px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
            {languageLabel(lang)}
          </span>
          <CopyButton
            text={contents.state === "ready" ? contents.text : undefined}
          />
        </div>
      </div>
      {contents.state === "ready" ? (
        <HighlightedCode text={contents.text} path={path} />
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center gap-2 bg-muted/20 p-6 text-xs text-muted-foreground">
          {contents.state === "loading" ? (
            <>
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Loading {path.split("/").pop()}…
            </>
          ) : (
            <span className="inline-flex items-center gap-1.5 text-destructive">
              <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
              Couldn't load <span className="font-mono">{path}</span>:{" "}
              {contents.message}
            </span>
          )}
        </div>
      )}
    </div>
  );
}

/** Read `path` through `read`, following a promise until it settles. */
function useContents(path: string, read: FileSource["read"]): Contents {
  // Memoized, so an asynchronous read starts once per file, not per render.
  const attempt = useMemo((): string | Promise<string> | Error => {
    try {
      return read(path);
    } catch (e) {
      return e instanceof Error ? e : new Error(String(e));
    }
  }, [path, read]);
  const [settled, setSettled] = useState<{
    of: Promise<string>;
    value: string | Error;
  }>();

  useEffect(() => {
    if (!(attempt instanceof Promise)) return;
    let live = true;
    attempt.then(
      (value) => live && setSettled({ of: attempt, value }),
      (e: unknown) =>
        live &&
        setSettled({
          of: attempt,
          value: e instanceof Error ? e : new Error(String(e)),
        }),
    );
    return () => {
      live = false;
    };
  }, [attempt]);

  const value =
    attempt instanceof Promise
      ? settled?.of === attempt
        ? settled.value
        : undefined
      : attempt;
  if (value === undefined) return { state: "loading" };
  if (value instanceof Error) return { state: "error", message: value.message };
  return { state: "ready", text: value };
}

/** The plain lines at once, the highlighted ones as soon as Shiki has them. */
function useHighlighted(text: string, path: string): CodeLine[] {
  const lang = languageFor(path);
  const plain = useMemo(() => plainLines(text), [text]);
  const [highlighted, setHighlighted] = useState<{
    of: string;
    lines: CodeLine[];
  }>();
  const key = `${lang}\0${text}`;

  useEffect(() => {
    let live = true;
    highlightLines(text, lang).then(
      (lines) => live && setHighlighted({ of: key, lines }),
      // Highlighting is an enhancement: on failure the plain text stays.
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [text, lang, key]);

  return highlighted?.of === key ? highlighted.lines : plain;
}

function HighlightedCode({ text, path }: { text: string; path: string }) {
  const lines = useHighlighted(text, path);
  return (
    <pre className="trestle-code min-h-0 flex-1 overflow-auto bg-muted/20 py-3 font-mono text-xs leading-relaxed text-foreground">
      <code className="block min-w-max">
        {lines.map((line, i) => (
          // Lines have no identity beyond their position.
          // biome-ignore lint/suspicious/noArrayIndexKey: see above.
          <span key={i} className="flex">
            <span
              aria-hidden="true"
              className="w-12 shrink-0 select-none pr-4 text-right text-muted-foreground/60"
            >
              {i + 1}
            </span>
            <span className="whitespace-pre pr-4">
              {line.length === 0 ? "​" : line.map(tokenSpan)}
            </span>
          </span>
        ))}
      </code>
    </pre>
  );
}

function tokenSpan(token: CodeLine[number], i: number): ReactNode {
  if (!token.light && !token.dark) return token.content;
  return (
    <span
      // Tokens have no identity beyond their position.
      key={i}
      className="trestle-code-token"
      style={
        {
          "--shiki-light": token.light,
          "--shiki-dark": token.dark,
        } as CSSProperties
      }
    >
      {token.content}
    </span>
  );
}

function CopyButton({ text }: { text: string | undefined }) {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    if (text === undefined) return;
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <button
      type="button"
      className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50"
      onClick={copy}
      disabled={text === undefined}
      aria-label="Copy file contents"
      title="Copy file contents"
    >
      {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
    </button>
  );
}
