// Syntax highlighting for rendered files, with Shiki. Everything Shiki is
// imported dynamically, on the first file shown, so neither the editor's nor
// the embedded diagram's initial bundle carries it. Each token gets both the
// light and the dark theme's color (`--shiki-light` / `--shiki-dark`); the
// stylesheet picks one under `.dark`, so no re-highlight on a theme switch.

import type { HighlighterCore, LanguageRegistration } from "shiki/core";

export type CodeLanguage =
  | "yaml"
  | "toml"
  | "json"
  | "shellscript"
  | "python"
  | "properties"
  | "ini"
  | "sql"
  | "dotenv"
  | "dockerfile"
  | "nginx"
  | "markdown"
  | "plaintext";

type Grammar = () => Promise<{ default: LanguageRegistration[] }>;

const GRAMMARS: Record<Exclude<CodeLanguage, "plaintext">, Grammar> = {
  yaml: () => import("shiki/langs/yaml.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  shellscript: () => import("shiki/langs/shellscript.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  properties: () => import("shiki/langs/properties.mjs"),
  ini: () => import("shiki/langs/ini.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  dotenv: () => import("shiki/langs/dotenv.mjs"),
  dockerfile: () => import("shiki/langs/dockerfile.mjs"),
  nginx: () => import("shiki/langs/nginx.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
};

const EXTENSIONS: Record<string, CodeLanguage> = {
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  json: "json",
  sh: "shellscript",
  bash: "shellscript",
  py: "python",
  properties: "properties",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  sql: "sql",
  env: "dotenv",
  md: "markdown",
};

const LABELS: Record<CodeLanguage, string> = {
  yaml: "YAML",
  toml: "TOML",
  json: "JSON",
  shellscript: "Shell",
  python: "Python",
  properties: "Properties",
  ini: "INI",
  sql: "SQL",
  dotenv: ".env",
  dockerfile: "Dockerfile",
  nginx: "nginx",
  markdown: "Markdown",
  plaintext: "Text",
};

/**
 * The language a file is written in, from its name: the extension (a
 * template's `.jinja` suffix is looked through), or a well-known basename.
 */
export function languageFor(path: string): CodeLanguage {
  const name = (path.split("/").pop() ?? path).replace(/\.j(inja)?2?$/, "");
  if (/^dockerfile$/i.test(name) || /\.dockerfile$/i.test(name)) {
    return "dockerfile";
  }
  if (name === ".env" || name.startsWith(".env.")) return "dotenv";
  if (name === "nginx.conf") return "nginx";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "plaintext";
  return EXTENSIONS[name.slice(dot + 1).toLowerCase()] ?? "plaintext";
}

/** A short display name for a language (the viewer's badge). */
export function languageLabel(lang: CodeLanguage): string {
  return LABELS[lang];
}

/** A run of text in one color; without colors it takes the text color. */
export interface CodeToken {
  content: string;
  light?: string;
  dark?: string;
}

export type CodeLine = CodeToken[];

/** The unhighlighted lines, drawn until (or instead of) the highlighted ones. */
export function plainLines(code: string): CodeLine[] {
  return trimFinalNewline(code)
    .split("\n")
    .map((content) => (content ? [{ content }] : []));
}

let highlighter: Promise<HighlighterCore> | undefined;

function loadHighlighter(): Promise<HighlighterCore> {
  highlighter ??= Promise.all([
    import("shiki/core"),
    import("shiki/engine/javascript"),
  ])
    .then(([{ createHighlighterCore }, { createJavaScriptRegexEngine }]) =>
      createHighlighterCore({
        themes: [
          import("shiki/themes/github-light.mjs"),
          import("shiki/themes/github-dark.mjs"),
        ],
        langs: [],
        engine: createJavaScriptRegexEngine(),
      }),
    )
    .catch((e: unknown) => {
      // Let a later file try again (e.g. after a flaky chunk load).
      highlighter = undefined;
      throw e;
    });
  return highlighter;
}

/** Highlight `code` as `lang`: one token list per line, colored for both themes. */
export async function highlightLines(
  code: string,
  lang: CodeLanguage,
): Promise<CodeLine[]> {
  if (lang === "plaintext") return plainLines(code);
  const shiki = await loadHighlighter();
  if (!shiki.getLoadedLanguages().includes(lang)) {
    await shiki.loadLanguage((await GRAMMARS[lang]()).default);
  }
  const lines = shiki.codeToTokensWithThemes(trimFinalNewline(code), {
    lang,
    themes: { light: "github-light", dark: "github-dark" },
  });
  return lines.map((line) =>
    line.map((token) => ({
      content: token.content,
      light: token.variants.light?.color,
      dark: token.variants.dark?.color,
    })),
  );
}

/** A file's trailing newline would otherwise draw an empty last line. */
function trimFinalNewline(code: string): string {
  return code.endsWith("\n") ? code.slice(0, -1) : code;
}
