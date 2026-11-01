// The Notes window's editor: CodeMirror with a live preview of the
// Markdown. The file's text is exactly what's typed; the preview only
// hides marks (`#`, `**`, a link's address) on lines the cursor isn't on,
// draws bullets and turns `- [ ]` into a checkbox that can be clicked.
//
// Built into app/ui/vendor/editor.js by `pnpm build`; notes.js uses it as
// the global `NoteEditor`.

import { Compartment, EditorState, Prec, Transaction, Annotation } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  WidgetType,
  keymap,
  placeholder as placeholderText,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentLess, indentMore } from "@codemirror/commands";
import { markdownKeymap, markdownLanguage } from "@codemirror/lang-markdown";
import { HighlightStyle, LanguageSupport, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { tags } from "@lezer/highlight";

/** Marks a change that came from the file, not from typing. */
const fromFile = Annotation.define();

// ---------------------------------------------------------------- the preview

class Bullet extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-bullet";
    span.textContent = "•";
    return span;
  }
}

class Checkbox extends WidgetType {
  constructor(checked) {
    super();
    this.checked = checked;
  }
  eq(other) {
    return other.checked === this.checked;
  }
  toDOM() {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "cm-task";
    box.checked = this.checked;
    box.setAttribute("aria-label", this.checked ? "Done" : "Not done");
    return box;
  }
  ignoreEvent() {
    return false;
  }
}

const hide = Decoration.replace({});
const bullet = Decoration.replace({ widget: new Bullet() });
const lineClass = (cls) => Decoration.line({ class: cls });
const markClass = (cls) => Decoration.mark({ class: cls });

/** The lines the selection touches, while the editor has focus: their
 *  marks show, so what's being edited is what's in the file. */
function activeLines(view) {
  const lines = new Set();
  if (!view.hasFocus) return lines;
  const doc = view.state.doc;
  for (const r of view.state.selection.ranges) {
    const first = doc.lineAt(r.from).number;
    const last = doc.lineAt(r.to).number;
    for (let n = first; n <= last; n++) lines.add(n);
  }
  return lines;
}

function build(view) {
  const all = [];
  const atomic = [];
  const doc = view.state.doc;
  const active = activeLines(view);
  const isActive = (from, to = from) => {
    const a = doc.lineAt(from).number;
    const b = doc.lineAt(to).number;
    for (let n = a; n <= b; n++) if (active.has(n)) return true;
    return false;
  };
  const replace = (from, to, deco = hide) => {
    if (to <= from) return;
    all.push(deco.range(from, to));
    atomic.push(deco.range(from, to));
  };
  const eachLine = (from, to, deco) => {
    for (let pos = from; pos <= to; ) {
      const line = doc.lineAt(pos);
      all.push(deco.range(line.from));
      pos = line.to + 1;
    }
  };

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(view.state).iterate({
      from,
      to,
      enter(node) {
        const name = node.name;
        const parent = node.node.parent?.name;
        const heading = /^(ATX|Setext)Heading(\d)$/.exec(name);
        if (heading) {
          eachLine(node.from, node.to, lineClass(`cm-h cm-h${heading[2]}`));
          return;
        }
        switch (name) {
          case "HeaderMark":
            if (parent?.startsWith("ATX") && !isActive(node.from)) {
              const space = doc.sliceString(node.to, node.to + 1) === " " ? 1 : 0;
              // A closing `##` hides on its own; the opening one takes its space.
              replace(node.from, node.to + (node.from === doc.lineAt(node.from).from ? space : 0));
            }
            return;
          case "EmphasisMark":
          case "StrikethroughMark":
            if (!isActive(node.from)) replace(node.from, node.to);
            return;
          case "InlineCode":
            all.push(markClass("cm-icode").range(node.from, node.to));
            return;
          case "CodeMark":
            if (parent === "InlineCode" && !isActive(node.from)) replace(node.from, node.to);
            return;
          case "FencedCode":
          case "CodeBlock":
            eachLine(node.from, node.to, lineClass("cm-fence"));
            return;
          case "Blockquote":
            eachLine(node.from, node.to, lineClass("cm-quote"));
            return;
          case "QuoteMark":
            if (!isActive(node.from)) {
              const space = doc.sliceString(node.to, node.to + 1) === " " ? 1 : 0;
              replace(node.from, node.to + space);
            }
            return;
          case "HorizontalRule":
            all.push(lineClass("cm-hr").range(doc.lineAt(node.from).from));
            if (!isActive(node.from)) replace(node.from, node.to);
            return;
          case "Link": {
            const marks = node.node.getChildren("LinkMark");
            if (marks.length >= 2) {
              all.push(markClass("cm-link").range(marks[0].to, marks[1].from));
            }
            if (isActive(node.from, node.to)) return false;
            // `[`, then `](url "title")` after the label.
            if (marks.length >= 2) {
              replace(marks[0].from, marks[0].to);
              replace(marks[1].from, node.to);
            }
            return false;
          }
          case "URL":
            if (parent !== "Link") all.push(markClass("cm-link").range(node.from, node.to));
            return;
          case "ListItem": {
            const mark = node.node.getChild("ListMark");
            const task = node.node.getChild("Paragraph")?.getChild("Task") ?? node.node.getChild("Task");
            const marker = task?.getChild("TaskMarker");
            if (mark && marker) {
              const checked = /x/i.test(doc.sliceString(marker.from, marker.to));
              // `- [ ] ` becomes the checkbox.
              const after = doc.sliceString(marker.to, marker.to + 1) === " " ? 1 : 0;
              replace(mark.from, marker.from);
              replace(marker.from, marker.to + after, Decoration.replace({ widget: new Checkbox(checked) }));
              if (checked) {
                const end = doc.lineAt(marker.from).to;
                if (end > marker.to + after) all.push(markClass("cm-done").range(marker.to + after, end));
              }
            } else if (mark && node.node.parent?.name === "BulletList") {
              replace(mark.from, mark.to, bullet);
            }
            return;
          }
          default:
            return;
        }
      },
    });
  }
  return { all: Decoration.set(all, true), atomic: Decoration.set(atomic, true) };
}

const preview = ViewPlugin.fromClass(
  class {
    constructor(view) {
      ({ all: this.decorations, atomic: this.atomic } = build(view));
    }
    update(u) {
      if (
        u.docChanged ||
        u.viewportChanged ||
        u.selectionSet ||
        u.focusChanged ||
        syntaxTree(u.startState) !== syntaxTree(u.state)
      ) {
        ({ all: this.decorations, atomic: this.atomic } = build(u.view));
      }
    }
  },
  {
    decorations: (p) => p.decorations,
    provide: (plugin) =>
      EditorView.atomicRanges.of((view) => view.plugin(plugin)?.atomic ?? Decoration.none),
    eventHandlers: {
      mousedown(e, view) {
        if (!(e.target instanceof HTMLInputElement) || !e.target.classList.contains("cm-task")) return false;
        e.preventDefault();
        if (view.state.readOnly) return true;
        // The widget replaced `[ ] `: its `[` is where it starts.
        const at = view.posAtDOM(e.target);
        const text = view.state.doc.sliceString(at, at + 3);
        if (!/^\[[ xX]\]$/.test(text)) return true;
        view.dispatch({
          changes: { from: at + 1, to: at + 2, insert: text[1] === " " ? "x" : " " },
          userEvent: "input.toggle",
        });
        return true;
      },
    },
  },
);

// ---------------------------------------------------------------- looks

const highlight = HighlightStyle.define([
  { tag: tags.strong, fontWeight: "650" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  { tag: tags.heading, fontWeight: "650" },
  { tag: tags.processingInstruction, color: "var(--muted)" },
  { tag: tags.url, color: "var(--muted)" },
  { tag: tags.monospace, fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" },
  { tag: tags.quote, color: "var(--muted)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "15px", color: "var(--text)", backgroundColor: "transparent" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "inherit", lineHeight: "1.6", overflow: "auto" },
  ".cm-content": {
    maxWidth: "720px",
    margin: "0 auto",
    padding: "22px 28px 64px",
    caretColor: "var(--accent)",
    width: "100%",
    boxSizing: "border-box",
  },
  ".cm-line": { padding: "0" },
  ".cm-placeholder": { color: "var(--muted)" },
  ".cm-h": { fontWeight: "650", lineHeight: "1.3" },
  ".cm-h1": { fontSize: "1.6em", paddingTop: "0.35em", paddingBottom: "0.1em" },
  ".cm-h2": { fontSize: "1.3em", paddingTop: "0.45em" },
  ".cm-h3": { fontSize: "1.1em", paddingTop: "0.45em" },
  ".cm-h4, .cm-h5, .cm-h6": { fontSize: "1em", paddingTop: "0.35em" },
  ".cm-bullet": { display: "inline-block", width: "1ch", color: "var(--muted)", textAlign: "center" },
  ".cm-task": {
    width: "15px",
    height: "15px",
    margin: "0 7px 0 0",
    verticalAlign: "-2px",
    accentColor: "var(--accent)",
    cursor: "pointer",
  },
  ".cm-done": { color: "var(--muted)", textDecoration: "line-through" },
  ".cm-icode": {
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    fontSize: "0.9em",
    background: "var(--surface-2)",
    borderRadius: "4px",
    padding: "1px 3px",
  },
  ".cm-fence": {
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    fontSize: "0.9em",
    background: "var(--surface-2)",
  },
  ".cm-quote": { borderLeft: "3px solid var(--border)", paddingLeft: "12px !important", color: "var(--muted)" },
  ".cm-hr": { borderBottom: "1px solid var(--border)", lineHeight: "0.8" },
  ".cm-link": { color: "var(--accent)" },
});

// ---------------------------------------------------------------- keys

/** Wraps the selection in `mark` (bold, italic), or takes it off. */
function wrap(mark) {
  return (view) => {
    const changes = view.state.changeByRange((r) => {
      const doc = view.state.doc;
      const n = mark.length;
      const before = doc.sliceString(r.from - n, r.from);
      const after = doc.sliceString(r.to, r.to + n);
      if (before === mark && after === mark) {
        return {
          changes: [{ from: r.from - n, to: r.from }, { from: r.to, to: r.to + n }],
          range: { anchor: r.from - n, head: r.to - n },
        };
      }
      return {
        changes: [{ from: r.from, insert: mark }, { from: r.to, insert: mark }],
        range: { anchor: r.from + n, head: r.to + n },
      };
    });
    view.dispatch(view.state.update(changes, { userEvent: "input.format" }));
    return true;
  };
}

// ---------------------------------------------------------------- the editor

/**
 * Makes an editor in `parent`.
 * options: placeholder, label (its accessible name), onChange(), onSend(),
 * onPaste(text) (true when it handled the paste).
 */
export function create(parent, options = {}) {
  const readOnly = new Compartment();
  const editable = new Compartment();
  const extensions = () => [
    history(),
    EditorView.lineWrapping,
    // Markdown alone: no other languages inside code blocks.
    new LanguageSupport(markdownLanguage),
    syntaxHighlighting(highlight),
    preview,
    theme,
    placeholderText(options.placeholder ?? ""),
    readOnly.of(EditorState.readOnly.of(false)),
    editable.of(EditorView.editable.of(true)),
    EditorView.contentAttributes.of({
      "aria-label": options.label ?? "Note",
      spellcheck: "true",
      autocapitalize: "sentences",
    }),
    Prec.highest(
      keymap.of([
        { key: "Mod-Enter", run: () => (options.onSend?.(), true) },
        { key: "Mod-b", run: wrap("**") },
        { key: "Mod-i", run: wrap("*") },
        { key: "Tab", run: indentMore },
        { key: "Shift-Tab", run: indentLess },
      ]),
    ),
    keymap.of([...markdownKeymap, ...historyKeymap, ...defaultKeymap.filter((k) => k.key !== "Mod-Enter")]),
    EditorView.updateListener.of((u) => {
      if (u.docChanged && !u.transactions.some((t) => t.annotation(fromFile))) options.onChange?.();
    }),
    EditorView.domEventHandlers({
      paste(e) {
        const text = e.clipboardData?.getData("text/plain");
        return !!(text && options.onPaste?.(text));
      },
    }),
  ];
  const view = new EditorView({ parent, state: EditorState.create({ doc: "", extensions: extensions() }) });

  return {
    view,
    get text() {
      return view.state.doc.toString();
    },
    /** Another note: its text, with a fresh undo history. */
    load(text) {
      view.setState(EditorState.create({ doc: text, extensions: extensions() }));
    },
    /** The same note, changed on disk: the new text, keeping the cursor
     *  where it can and leaving undo alone. */
    reload(text) {
      const doc = view.state.doc;
      if (doc.toString() === text) return;
      const head = Math.min(view.state.selection.main.head, text.length);
      view.dispatch({
        changes: { from: 0, to: doc.length, insert: text },
        selection: { anchor: head },
        annotations: [fromFile.of(true), Transaction.addToHistory.of(false)],
      });
    },
    focus() {
      view.focus();
    },
    get hasFocus() {
      return view.hasFocus;
    },
    /** The selection as offsets into the text. */
    get selection() {
      const r = view.state.selection.main;
      return { from: r.from, to: r.to };
    },
    /** Replaces from–to with `text` as an edit ⌘Z undoes, and selects it. */
    replace(from, to, text, { select = true } = {}) {
      view.dispatch({
        changes: { from, to, insert: text },
        selection: select ? { anchor: from, head: from + text.length } : { anchor: from + text.length },
        userEvent: "input.replace",
        scrollIntoView: true,
      });
    },
    set readOnly(on) {
      view.dispatch({
        effects: [
          readOnly.reconfigure(EditorState.readOnly.of(on)),
          editable.reconfigure(EditorView.editable.of(!on)),
        ],
      });
    },
    get readOnly() {
      return view.state.readOnly;
    },
    /** Whether the end of the note is in view. */
    get atEnd() {
      const s = view.scrollDOM;
      return s.scrollTop + s.clientHeight >= s.scrollHeight - 24;
    },
    scrollToEnd() {
      view.scrollDOM.scrollTop = view.scrollDOM.scrollHeight;
    },
    /** Puts the cursor at the end. */
    toEnd() {
      view.dispatch({ selection: { anchor: view.state.doc.length }, scrollIntoView: true });
    },
  };
}
