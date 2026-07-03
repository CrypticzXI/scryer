import { useEffect, useRef } from "react";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  lineNumbers,
  keymap,
} from "@codemirror/view";
import { EditorState, StateEffect, StateField, type Extension } from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import {
  defaultHighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
  type StreamParser,
} from "@codemirror/language";
import { defaultKeymap, indentWithTab } from "@codemirror/commands";
import { useTheme } from "next-themes";
import "@fontsource-variable/jetbrains-mono";
import { CODE_FONT } from "@/lib/fonts";
import { isDarkTheme } from "@/lib/theme";

export type CodeEditorLanguage = "plain" | "javascript" | "rego" | "shell";

export type CodeEditorDiagnostic = {
  line: number;
  column?: number | null;
  message?: string;
};

export type CodeEditorProps = {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  height?: string;
  minLines?: number;
  maxLines?: number;
  diagnostics?: CodeEditorDiagnostic[];
  language?: CodeEditorLanguage;
};

const regoKeywords = new Set([
  "as",
  "contains",
  "default",
  "else",
  "every",
  "false",
  "if",
  "import",
  "in",
  "not",
  "null",
  "package",
  "some",
  "true",
  "with",
]);

const regoBuiltins = new Set([
  "abs",
  "all",
  "any",
  "array",
  "base64",
  "bits",
  "ceil",
  "concat",
  "contains",
  "count",
  "data",
  "endswith",
  "floor",
  "glob",
  "input",
  "is_array",
  "is_boolean",
  "is_null",
  "is_number",
  "is_object",
  "is_set",
  "is_string",
  "json",
  "lower",
  "max",
  "min",
  "numbers",
  "object",
  "regex",
  "replace",
  "round",
  "set",
  "sort",
  "split",
  "sprintf",
  "startswith",
  "strings",
  "sum",
  "time",
  "to_number",
  "trim",
  "type_name",
  "upper",
  "urlquery",
]);

const regoParser: StreamParser<null> = {
  name: "rego",
  token(stream) {
    if (stream.eatSpace()) return null;
    if (stream.match("#")) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match(/"(?:[^"\\]|\\.)*"?/)) return "string";
    if (stream.match(/`[^`]*`?/)) return "string";
    if (stream.match(/\d+(?:\.\d+)?/)) return "number";
    if (stream.match(/[{}()[\],.;:]/)) return "punctuation";
    if (stream.match(/==|!=|<=|>=|:=|[_+\-*/%<>=|&!]+/)) return "operator";

    const word = stream.match(/[A-Za-z_][A-Za-z0-9_]*/);
    if (word && word !== true) {
      const value = word[0];
      if (regoKeywords.has(value)) return "keyword";
      if (regoBuiltins.has(value)) return "variableName.special";
      return "variableName";
    }

    stream.next();
    return null;
  },
};

const setDiagnosticsEffect = StateEffect.define<CodeEditorDiagnostic[]>();

function diagnosticDecorations(
  state: EditorState,
  diagnostics: CodeEditorDiagnostic[],
): DecorationSet {
  const decorations = diagnostics
    .filter((diagnostic) => Number.isFinite(diagnostic.line))
    .sort((a, b) => a.line - b.line)
    .flatMap((diagnostic) => {
      if (diagnostic.line < 1 || diagnostic.line > state.doc.lines) {
        return [];
      }

      const line = state.doc.line(diagnostic.line);
      const column = diagnostic.column && Number.isFinite(diagnostic.column)
        ? Math.max(1, diagnostic.column)
        : 1;
      const from = Math.min(line.to, line.from + column - 1);
      const to = Math.max(from + 1, Math.min(line.to, line.to));
      const attributes = diagnostic.message ? { title: diagnostic.message } : undefined;

      return [
        Decoration.line({ class: "cm-code-diagnostic-line" }).range(line.from),
        Decoration.mark({ attributes, class: "cm-code-diagnostic" }).range(from, to),
      ];
    });

  return Decoration.set(decorations, true);
}

const diagnosticField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(decorations, transaction) {
    for (const effect of transaction.effects) {
      if (effect.is(setDiagnosticsEffect)) {
        return diagnosticDecorations(transaction.state, effect.value);
      }
    }

    if (transaction.docChanged) {
      return decorations.map(transaction.changes);
    }

    return decorations;
  },
  provide: (field) => EditorView.decorations.from(field),
});

const diagnosticTheme = EditorView.baseTheme({
  ".cm-code-diagnostic-line": {
    backgroundColor: "rgba(255, 88, 112, 0.08)",
  },
  ".cm-code-diagnostic": {
    textDecorationColor: "var(--scry-danger-text)",
    textDecorationLine: "underline",
    textDecorationSkipInk: "none",
    textDecorationStyle: "wavy",
    textDecorationThickness: "1.5px",
    textUnderlineOffset: "3px",
  },
});

const lightTheme = EditorView.theme({
  "&": { backgroundColor: "var(--background)", color: "var(--foreground)" },
  ".cm-gutters": { backgroundColor: "var(--muted)", borderRight: "1px solid var(--border)", paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "var(--accent)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
  ".cm-content": { fontFamily: CODE_FONT, paddingLeft: "8px" },
  ".cm-gutters .cm-gutter": { fontFamily: CODE_FONT },
});

const scryerDark = EditorView.theme({
  "&": { backgroundColor: "#0a0e1a", color: "#d4d4d8" },
  ".cm-content": { fontFamily: CODE_FONT, caretColor: "#5b64ff", paddingLeft: "8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#5b64ff" },
  ".cm-gutters": { backgroundColor: "#0a0e1a", color: "#3f3f46", borderRight: "1px solid #273255", fontFamily: CODE_FONT, paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "rgba(255,255,255,0.03)", color: "#71717a" },
  ".cm-activeLine": { backgroundColor: "rgba(255,255,255,0.03)" },
  "&.cm-focused": { outline: "2px solid hsl(var(--ring))" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(91,100,255,0.2)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(91,100,255,0.3)" },
  ".cm-line": { padding: "0 4px" },
}, { dark: true });

const prideTheme = EditorView.theme({
  "&": { backgroundColor: "#100d20", color: "#fff0fb" },
  ".cm-content": { fontFamily: CODE_FONT, caretColor: "#ff5ca8", paddingLeft: "8px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#ff5ca8" },
  ".cm-gutters": { backgroundColor: "#0d0a1b", color: "#9587b7", borderRight: "1px solid rgba(255,255,255,0.14)", fontFamily: CODE_FONT, paddingRight: "8px" },
  ".cm-activeLineGutter": { backgroundColor: "rgba(255,255,255,0.05)", color: "#ffd8f0" },
  ".cm-activeLine": { backgroundColor: "rgba(255,255,255,0.04)" },
  "&.cm-focused": { outline: "2px solid var(--ring)" },
  ".cm-selectionBackground, ::selection": { backgroundColor: "rgba(255,92,168,0.24)" },
  "&.cm-focused .cm-selectionBackground": { backgroundColor: "rgba(255,92,168,0.35)" },
  ".cm-line": { padding: "0 4px" },
}, { dark: true });

function languageExtensions(language: CodeEditorLanguage): Extension[] {
  if (language === "javascript") {
    return [javascript()];
  }

  if (language === "rego") {
    return [StreamLanguage.define(regoParser)];
  }

  if (language === "shell") {
    return [StreamLanguage.define(shell)];
  }

  return [];
}

export default function CodeEditor({
  id,
  value,
  onChange,
  readOnly = false,
  height = "320px",
  minLines,
  maxLines,
  diagnostics = [],
  language = "plain",
}: CodeEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const { resolvedTheme } = useTheme();
  const lineCount = value.split("\n").length;
  const lineHeightPx = 20;
  const computedHeight =
    minLines && maxLines
      ? `${Math.min(Math.max(lineCount, minLines), maxLines) * lineHeightPx}px`
      : height;

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    if (!containerRef.current) return;

    const usePrideTheme = resolvedTheme === "pride";
    const useDarkTheme = isDarkTheme(resolvedTheme);

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        onChangeRef.current(update.state.doc.toString());
      }
    });
    const highlightStyle =
      usePrideTheme || useDarkTheme ? oneDarkHighlightStyle : defaultHighlightStyle;

    const extensions = [
      lineNumbers(),
      ...languageExtensions(language),
      syntaxHighlighting(highlightStyle),
      keymap.of([...defaultKeymap, indentWithTab]),
      updateListener,
      diagnosticField,
      diagnosticTheme,
      EditorView.lineWrapping,
      usePrideTheme ? prideTheme : useDarkTheme ? scryerDark : lightTheme,
    ];

    if (readOnly) {
      extensions.push(EditorState.readOnly.of(true));
    }

    const state = EditorState.create({
      doc: value,
      extensions,
    });

    const view = new EditorView({
      state,
      parent: containerRef.current,
    });

    viewRef.current = view;
    if (diagnostics.length > 0) {
      view.dispatch({
        effects: setDiagnosticsEffect.of(diagnostics),
      });
    }

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Recreate editor when theme, language, or read-only behavior changes.
    // Value and diagnostics are synchronized by dedicated effects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedTheme, readOnly, language]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== value) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
      });
    }
  }, [value]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: setDiagnosticsEffect.of(diagnostics),
    });
  }, [diagnostics]);

  return (
    <div
      id={id}
      ref={containerRef}
      className="overflow-auto rounded-lg border border-border text-sm"
      style={{ height: computedHeight, minHeight: "120px" }}
    />
  );
}
