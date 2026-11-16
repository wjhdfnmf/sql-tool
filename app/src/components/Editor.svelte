<script lang="ts">
  import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from '@codemirror/autocomplete';
  import { defaultKeymap, history, historyKeymap, indentWithTab, redo as redoCommand, selectAll as selectAllCommand, undo as undoCommand } from '@codemirror/commands';
  import { MariaSQL, MySQL, sql } from '@codemirror/lang-sql';
  import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from '@codemirror/language';
  import { highlightSelectionMatches, openSearchPanel, searchKeymap } from '@codemirror/search';
  import { EditorState, Prec } from '@codemirror/state';
  import {
    drawSelection,
    EditorView,
    highlightActiveLine,
    highlightActiveLineGutter,
    keymap,
    lineNumbers,
  } from '@codemirror/view';
  import { tags } from '@lezer/highlight';
  import { onMount } from 'svelte';

  interface Props {
    value: string;
    mariadb: boolean;
    onchange: (text: string) => void;
    /** Ctrl/Cmd+Enter runs the statement at the cursor; with Shift, the whole script. */
    onrun: (all: boolean) => void;
  }

  let { value, mariadb, onchange, onrun }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;

  // Colours come from CSS variables so the editor follows light and dark mode.
  const theme = EditorView.theme({
    '&': { height: '100%', fontSize: '13px', backgroundColor: 'var(--bg)', color: 'var(--text)' },
    '.cm-scroller': { fontFamily: 'var(--mono)', lineHeight: '1.5' },
    '.cm-content': { caretColor: 'var(--text)' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
    '.cm-gutters': { backgroundColor: 'var(--panel)', color: 'var(--muted)', border: 'none', borderRight: '1px solid var(--border)' },
    '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--selection) 45%, transparent)' },
    '.cm-activeLineGutter': { backgroundColor: 'var(--panel-2)' },
    '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground': {
      backgroundColor: 'var(--selection)',
    },
    '.cm-selectionMatch': { backgroundColor: 'color-mix(in srgb, var(--accent) 22%, transparent)' },
    '&.cm-focused': { outline: 'none' },
    '.cm-tooltip': { backgroundColor: 'var(--bg)', color: 'var(--text)', border: '1px solid var(--border)', borderRadius: '4px' },
    '.cm-tooltip-autocomplete > ul > li[aria-selected]': { backgroundColor: 'var(--accent)', color: 'var(--accent-text)' },
    '.cm-panels': { backgroundColor: 'var(--panel)', color: 'var(--text)', borderTop: '1px solid var(--border)' },
    '.cm-textfield, .cm-button': { backgroundColor: 'var(--bg)', color: 'var(--text)', border: '1px solid var(--border)', backgroundImage: 'none' },
  });

  const highlight = HighlightStyle.define([
    { tag: tags.keyword, color: 'var(--syn-keyword)', fontWeight: '600' },
    { tag: [tags.string, tags.special(tags.string)], color: 'var(--syn-string)' },
    { tag: [tags.number, tags.bool, tags.null], color: 'var(--syn-number)' },
    { tag: [tags.lineComment, tags.blockComment], color: 'var(--syn-comment)', fontStyle: 'italic' },
    { tag: [tags.typeName, tags.standard(tags.name)], color: 'var(--syn-name)' },
  ]);

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          autocompletion(),
          highlightSelectionMatches(),
          sql({ dialect: mariadb ? MariaSQL : MySQL, upperCaseKeywords: true }),
          syntaxHighlighting(highlight),
          theme,
          EditorView.contentAttributes.of({ 'aria-label': 'SQL editor' }),
          Prec.highest(
            keymap.of([
              { key: 'Mod-Enter', run: () => (onrun(false), true) },
              { key: 'Shift-Mod-Enter', run: () => (onrun(true), true) },
            ]),
          ),
          keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onchange(update.state.doc.toString());
          }),
        ],
      }),
    });
    return () => view?.destroy();
  });

  export function getText(): string {
    return view?.state.doc.toString() ?? value;
  }

  /** Cursor position in UTF-16 code units, which is what the backend expects. */
  export function getCursor(): number {
    return view?.state.selection.main.head ?? 0;
  }

  export function getSelection(): string {
    if (!view) return '';
    const { from, to } = view.state.selection.main;
    return view.state.sliceDoc(from, to);
  }

  /** Replaces the selection (or inserts at the cursor) and focuses the editor. */
  export function insert(text: string): void {
    if (!view) return;
    view.dispatch(view.state.replaceSelection(text));
    view.focus();
  }

  /** Replaces the whole document, as one undoable step. */
  export function replaceAll(text: string): void {
    if (!view) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    view.focus();
  }

  export function undo(): void {
    if (view) undoCommand(view);
    view?.focus();
  }

  export function redo(): void {
    if (view) redoCommand(view);
    view?.focus();
  }

  export function selectAll(): void {
    if (view) selectAllCommand(view);
    view?.focus();
  }

  export function openSearch(): void {
    if (view) openSearchPanel(view);
  }

  export function focus(): void {
    view?.focus();
  }
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
</style>
