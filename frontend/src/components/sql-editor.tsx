import { useEffect, useRef } from "react";
import { basicSetup } from "codemirror";
import { sql, StandardSQL } from "@codemirror/lang-sql";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { cn } from "@/lib/utils";

// Colors come from the app's CSS variables, so the editor follows light and dark themes.
const highlight = HighlightStyle.define([
  { tag: t.keyword, color: "hsl(var(--chart-4))", fontWeight: "600" },
  { tag: [t.string, t.special(t.string)], color: "hsl(var(--chart-2))" },
  { tag: [t.number, t.bool, t.null], color: "hsl(var(--chart-3))" },
  { tag: [t.lineComment, t.blockComment], color: "hsl(var(--muted-foreground))", fontStyle: "italic" },
  { tag: [t.operator, t.punctuation, t.paren], color: "hsl(var(--muted-foreground))" },
  { tag: [t.typeName, t.standard(t.name)], color: "hsl(var(--chart-1))" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "12.5px", backgroundColor: "transparent", color: "hsl(var(--foreground))" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "ui-monospace, 'SF Mono', Menlo, Consolas, monospace", lineHeight: "1.55" },
  ".cm-content": { caretColor: "hsl(var(--foreground))", padding: "8px 0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "hsl(var(--foreground))" },
  ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "hsl(var(--muted-foreground) / 0.7)" },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "hsl(var(--muted) / 0.45)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground": {
    backgroundColor: "hsl(var(--chart-1) / 0.22)",
  },
  ".cm-matchingBracket": { backgroundColor: "hsl(var(--chart-1) / 0.18)", outline: "none" },
  ".cm-tooltip": {
    backgroundColor: "hsl(var(--popover))",
    color: "hsl(var(--popover-foreground))",
    border: "1px solid hsl(var(--border))",
    borderRadius: "6px",
  },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "hsl(var(--accent))",
    color: "hsl(var(--accent-foreground))",
  },
});

const sqlLanguage = (schema: Record<string, readonly string[]>) => sql({ dialect: StandardSQL, schema, upperCaseKeywords: true });

/** CodeMirror 6 SQL editor. `schema` feeds table and column completion; Mod-Enter runs. */
export function SqlEditor({
  value,
  onChange,
  onRun,
  schema,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  onRun: () => void;
  schema: Record<string, readonly string[]>;
  className?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const language = useRef(new Compartment());
  // Latest callbacks, read by the editor's listeners without recreating the editor.
  const callbacks = useRef({ onChange, onRun });
  callbacks.current = { onChange, onRun };

  useEffect(() => {
    const editor = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          Prec.highest(
            keymap.of([
              {
                key: "Mod-Enter",
                run: () => {
                  callbacks.current.onRun();
                  return true;
                },
              },
            ]),
          ),
          basicSetup,
          language.current.of(sqlLanguage(schema)),
          syntaxHighlighting(highlight),
          theme,
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) callbacks.current.onChange(u.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
    // Created once; `value` and `schema` changes are applied by the effects below.
  }, []);

  useEffect(() => {
    view.current?.dispatch({ effects: language.current.reconfigure(sqlLanguage(schema)) });
  }, [schema]);

  // External value changes (e.g. "reset query") replace the document.
  useEffect(() => {
    const editor = view.current;
    if (editor && editor.state.doc.toString() !== value) {
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: value } });
    }
  }, [value]);

  return <div ref={host} className={cn("min-h-0 overflow-hidden", className)} />;
}
