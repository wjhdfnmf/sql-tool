<script lang="ts">
  import { adminQueries } from '../lib/admin';
  import { backend } from '../lib/backend';
  import { defaultRole } from '../lib/chart';
  import { GraphState } from '../lib/graph.svelte';
  import { layout, saveLayout } from '../lib/layout.svelte';
  import { separator, type MenuItem } from '../lib/menu';
  import { placeIn } from '../lib/place';
  import { keys } from '../lib/platform';
  import { reformatPiece, reformatSql, type KeywordCase, type ReformatStyle } from '../lib/reformat';
  import { settings } from '../lib/settings.svelte';
  import { app, type EditorTab, type WorkspaceState } from '../lib/state.svelte';
  import {
    asUiError,
    formatCount,
    formatDuration,
    quoteIdentifier,
    serverLabel,
    type Concern,
    type ObjectAction,
    type ObjectKind,
    type TreeTarget,
  } from '../lib/types';
  import ConfirmSqlDialog from './ConfirmSqlDialog.svelte';
  import Editor from './Editor.svelte';
  import GraphView from './GraphView.svelte';
  import HistoryPanel from './HistoryPanel.svelte';
  import InfoPane from './InfoPane.svelte';
  import Menu from './Menu.svelte';
  import Results from './Results.svelte';
  import SchemaEditor from './SchemaEditor.svelte';
  import SchemaTree from './SchemaTree.svelte';
  import SliceDialog, { type SliceSetup } from './SliceDialog.svelte';
  import TableDesigner from './TableDesigner.svelte';

  interface Props {
    workspace: WorkspaceState;
  }

  let { workspace }: Props = $props();

  let editors = $state<Record<string, Editor | undefined>>({});
  let editorHeight = $state(260);

  // The output put away while the editor is in use (a setting). 'open' is the
  // ordinary state; 'tucked' is hidden; 'peek' is shown because the pointer
  // went to the bottom, and goes away again when the pointer leaves unless
  // something in it was clicked.
  let outputMode = $state<'open' | 'peek' | 'tucked'>('open');
  let outputTimer: ReturnType<typeof setTimeout> | undefined;
  const autoHide = $derived(settings.current.autoHideOutput);
  const tucked = $derived(autoHide && outputMode === 'tucked');

  function setOutput(mode: 'open' | 'peek' | 'tucked', after = 0) {
    clearTimeout(outputTimer);
    if (!autoHide) return;
    if (after === 0) outputMode = mode;
    else outputTimer = setTimeout(() => (outputMode = mode), after);
  }

  /** The pointer left the output: one that was only being looked at goes away again. */
  function leaveOutput() {
    if (outputMode === 'peek') setOutput('tucked', 350);
  }

  /**
   * Switches "hide the output while editing" on or off without a trip to
   * Settings: a double-click on Output, or the keyboard. The output is left
   * in view either way, so what changed can be seen: switched on, it goes
   * away at the next click in the editor.
   */
  async function toggleAutoHide() {
    clearTimeout(outputTimer);
    workspace.showOutput = true;
    outputMode = 'open';
    try {
      await settings.save({ ...$state.snapshot(settings.current), autoHideOutput: !settings.current.autoHideOutput });
    } catch (e) {
      tab.report(asUiError(e).message);
    }
  }

  // A query that starts always brings its output into view.
  $effect(() => {
    if (tab.running) {
      clearTimeout(outputTimer);
      outputMode = 'open';
    }
  });
  let historyOpen = $state(false);
  let infoOpen = $state(false);
  let aside = $state<HTMLElement | undefined>();
  let strip = $state<HTMLDivElement | undefined>();
  let historyKey = $state(0);
  let main: HTMLDivElement;
  let contextMenu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null);
  /** Open drop, truncate or rename confirmation. */
  let danger = $state<{ action: ObjectAction; kind: ObjectKind; schema: string; name: string } | null>(null);
  /** Statements waiting for a go-ahead on a production connection. */
  let confirmation = $state<{ concerns: Concern[]; resolve: (go: boolean) => void } | null>(null);
  let closing = $state<EditorTab | null>(null);
  let closeDialog: HTMLDialogElement;

  const info = $derived(workspace.info);
  const tab = $derived(workspace.activeTab);
  /** The navigator shows when it is switched on, and always beside a designer. */
  const navigatorShown = $derived(workspace.showNavigator || !!tab.design);
  /**
   * The result a graph can be made from: the one in front in a query tab,
   * once it has finished, if it has rows and a column that can be charted.
   */
  const graphResult = $derived.by(() => {
    if (tab.design || tab.activeResult === 'messages') return null;
    const result = tab.results.find((r) => r.id === tab.activeResult);
    return result && result.done && result.rows > 0 && result.columns.some((c) => defaultRole(c.kind) !== null) ? result : null;
  });
  /** The graph builder is what the window shows. */
  const graphShown = $derived(
    // Only while the result it charts is still there: a tab whose query has been run again shows the query.
    workspace.graphOpen && tab.graph !== null && tab.results.some((r) => r.id === tab.graph?.resultId),
  );

  function toggleGraph() {
    if (graphShown) {
      closeGraph();
      return;
    }
    const result = graphResult;
    if (!result) return;
    // The same query run again keeps its graph; different columns start a new one.
    if (!tab.graph || !tab.graph.fits(result.columns)) tab.graph = new GraphState(result.id, result.columns);
    else if (tab.graph.resultId !== result.id) tab.graph.follow(result.id, result.columns);
    workspace.graphOpen = true;
  }

  function closeGraph() {
    workspace.graphOpen = false;
    // The columns the backend read out for the chart are let go; the setup stays on the tab.
    if (tab.graph) void backend.chartClose(tab.graph.resultId).catch(() => {});
  }

  /** The tab that queries run in: the active one, unless that is a designer. */
  const queryTab = $derived(tab.design ? (workspace.tabs.find((t) => !t.design) ?? null) : tab);

  /** Shows a message in the query tab's output. A designer tab has no output of its own. */
  function report(text: string, state: 'ok' | 'error' = 'error') {
    (queryTab ?? workspace.addTab()).report(text, state);
  }
  const mariadb = $derived(info.info.version.flavor === 'mariadb');
  const tls = $derived(info.info.tlsVersion?.replace('TLSv', 'TLS ') ?? null);
  const security = $derived(
    info.security === 'verified'
      ? { text: `Encrypted (${tls}), server verified`, level: 'good' }
      : info.security === 'pinned'
        ? { text: `Encrypted (${tls}), pinned certificate`, level: 'good' }
        : info.security === 'unverified'
          ? { text: `Encrypted (${tls}), server not verified`, level: 'caution' }
          : { text: 'Not encrypted', level: 'bad' },
  );

  const rowLimits: { label: string; value: number | null }[] = [
    { label: '1,000 rows', value: 1000 },
    { label: '10,000 rows', value: 10_000 },
    { label: '100,000 rows', value: 100_000 },
    { label: 'No limit', value: null },
  ];

  // On a production connection, risky statements wait for a go-ahead here.
  $effect(() => {
    workspace.confirmRun = (concerns) => new Promise((resolve) => (confirmation = { concerns, resolve }));
    return () => {
      workspace.confirmRun = null;
    };
  });

  // Remember the query tabs shortly after they change.
  $effect(() => {
    for (const t of workspace.tabs) {
      void t.text;
      void t.title;
      void t.filePath;
      void t.savedText;
    }
    const timer = setTimeout(() => workspace.saveTabs(), 800);
    return () => clearTimeout(timer);
  });

  function answerConfirmation(go: boolean) {
    confirmation?.resolve(go);
    confirmation = null;
  }

  // ---------- running ----------

  /** `script`: the selection if there is one, else everything. `statement`: the one under the cursor. */
  async function run(target: EditorTab, how: 'script' | 'statement' | 'explain') {
    const editor = editors[target.id];
    if (!editor) return;
    const selected = editor.getSelection();
    if (how === 'script') {
      await workspace.run(target, selected.trim() !== '' ? selected : editor.getText(), { kind: 'all' });
    } else {
      await workspace.run(target, editor.getText(), { kind: how === 'explain' ? 'explainAt' : 'at', offset: editor.getCursor() });
    }
    historyKey++;
  }

  /** The query a run in time slices is being set up for, while its dialog is open. */
  let slicing = $state<{ tab: EditorTab; sql: string } | null>(null);

  /** Opens the dialog for running the selection, or the whole tab, in time slices. */
  function sliceQuery() {
    const editor = editors[tab.id];
    if (!editor || tab.running) return;
    const selected = editor.getSelection();
    // One statement, without the semicolon that would end it.
    const sql = (selected.trim() !== '' ? selected : editor.getText()).trim().replace(/;\s*$/, '');
    if (sql === '') return;
    slicing = { tab, sql };
  }

  async function runSlices(setup: SliceSetup, stepSecs: number) {
    if (!slicing) return;
    const { tab: target, sql } = slicing;
    target.sliceSetup = setup;
    await workspace.runSliced(target, sql, setup.start, setup.end, stepSecs);
    historyKey++;
  }

  /** "about 40 s left", from milliseconds. */
  function timeLeft(ms: number | null): string {
    if (ms === null) return '';
    const seconds = Math.max(1, Math.round(ms / 1000));
    if (seconds < 90) return `about ${seconds} s left`;
    const minutes = Math.round(seconds / 60);
    return minutes < 90 ? `about ${minutes} min left` : `about ${(minutes / 60).toFixed(1)} h left`;
  }

  async function openAndRun(title: string, sql: string) {
    const created = workspace.addTab(sql, title);
    await workspace.run(created, sql, { kind: 'all' });
    historyKey++;
  }

  function openTable(schema: string, table: string) {
    void openAndRun(table, `SELECT * FROM ${quoteIdentifier(schema)}.${quoteIdentifier(table)} LIMIT 1000;`);
  }

  /** Makes a schema the default for every tab of this connection, and remembers it. */
  async function useSchema(schema: string) {
    try {
      await workspace.setDefaultSchema(schema);
    } catch (e) {
      report(asUiError(e).message);
    }
  }

  /** A tab shows its output once it has something to show; until then the editor has the room. */
  function hasOutput(t: EditorTab): boolean {
    return t.running || t.messages.length > 0 || t.results.length > 0;
  }

  // ---------- tabs ----------

  /** Goes to the first, previous, next or last tab. */
  function stepTab(to: 'first' | 'last' | -1 | 1) {
    const tabs = workspace.tabs;
    const at = tabs.findIndex((t) => t.id === workspace.activeTabId);
    const index = to === 'first' ? 0 : to === 'last' ? tabs.length - 1 : Math.min(Math.max(at + to, 0), tabs.length - 1);
    workspace.activeTabId = tabs[index].id;
  }

  function tabLabel(t: EditorTab): string {
    return t.title + ((t.filePath && t.dirty) || t.designDirty ? ' •' : '');
  }

  function showTabList(e: MouseEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    contextMenu = {
      items: workspace.tabs.map(
        (t): MenuItem => ({
          label: tabLabel(t) + (t.running ? '  (running)' : ''),
          checked: t.id === workspace.activeTabId,
          action: () => (workspace.activeTabId = t.id),
        }),
      ),
      x: rect.right - 240,
      y: rect.bottom + 2,
    };
  }

  // The tabs stay on one row; the one in front is kept in view.
  $effect(() => {
    void workspace.activeTabId;
    void workspace.tabs.length;
    const row = strip;
    if (!row) return;
    queueMicrotask(() => {
      const active = row.querySelector<HTMLElement>('.editor-tab.active');
      if (!active) return;
      const left = active.offsetLeft - row.offsetLeft;
      if (left < row.scrollLeft) row.scrollLeft = left - 8;
      else if (left + active.offsetWidth > row.scrollLeft + row.clientWidth) {
        row.scrollLeft = left + active.offsetWidth - row.clientWidth + 8;
      }
    });
  });

  // ---------- script files ----------

  async function openScript() {
    try {
      const file = await backend.scriptOpen(info.id);
      if (!file) return;
      const created = workspace.addTab(file.text, file.name);
      created.filePath = file.path;
    } catch (e) {
      report(asUiError(e).message);
    }
  }

  async function saveScript(target: EditorTab, saveAs = false): Promise<boolean> {
    // A designer is not a script; there is nothing to write to a file.
    if (target.design) return false;
    const text = editors[target.id]?.getText() ?? target.text;
    try {
      const suggested = target.filePath ? target.title : `${target.title}.sql`;
      let saved;
      try {
        saved = await backend.scriptSave(info.id, saveAs ? null : target.filePath, suggested, text);
      } catch (e) {
        // The file is not one the backend knows the user chose (its list was
        // cleared, or the tab came from an older version). Ask where instead.
        if (saveAs || asUiError(e).kind !== 'not-approved') throw e;
        saved = await backend.scriptSave(info.id, null, suggested, text);
      }
      if (!saved) return false;
      target.filePath = saved.path;
      target.title = saved.name;
      target.savedText = text;
      return true;
    } catch (e) {
      target.report(asUiError(e).message);
      return false;
    }
  }

  function requestClose(target: EditorTab) {
    if (target.design) {
      // A designer with changes that were never applied asks first.
      if (target.designDirty) {
        closing = target;
        closeDialog.showModal();
      } else {
        workspace.closeTab(target);
      }
      return;
    }
    // Only a file on disk can lose work silently; scratch tabs just close.
    // Asked of the editor itself: `target.text` trails typing by a moment.
    const text = editors[target.id]?.getText() ?? target.text;
    if (target.filePath && text !== target.savedText) {
      closing = target;
      closeDialog.showModal();
    } else {
      workspace.closeTab(target);
    }
  }

  async function finishClose(save: boolean) {
    const target = closing;
    closeDialog.close();
    closing = null;
    if (!target) return;
    if (save && !(await saveScript(target))) return;
    workspace.closeTab(target);
  }

  // ---------- editing ----------

  function reformat() {
    const editor = editors[tab.id];
    if (!editor) return;
    // What is selected is reformatted and nothing else is touched. With nothing selected, it is the whole text.
    const selection = editor.getSelection();
    const partial = selection.trim() !== '';
    const source = partial ? selection : editor.getText();
    if (source.trim() === '') return;
    if (/^\s*delimiter\s/im.test(source)) {
      report(
        partial
          ? 'The selection has a DELIMITER line in it, which cannot be reformatted. Select the statements without it.'
          : 'A script that uses DELIMITER cannot be reformatted as a whole. Select the part to reformat.',
      );
      return;
    }
    try {
      const { reformatStyle, reformatKeywords, editorTabSize } = settings.current;
      const [style, keywords] = [reformatStyle as ReformatStyle, reformatKeywords as KeywordCase];
      if (partial) editor.replaceSelection(reformatPiece(selection, editor.getLineBeforeSelection(), mariadb, style, keywords, editorTabSize));
      else editor.replaceAll(reformatSql(source, mariadb, style, keywords, editorTabSize) + '\n');
    } catch (e) {
      const reason = e instanceof Error ? e.message.split('\n')[0] : String(e);
      report(
        partial
          ? `Could not reformat the selection: ${reason}. A selection has to be SQL that stands on its own: whole statements, or a piece with its brackets closed.`
          : `Could not reformat: ${reason}`,
      );
    }
  }

  // ---------- schema objects ----------

  async function createFromTemplate(kind: ObjectKind, schema?: string) {
    try {
      const target = schema ?? workspace.selected?.schema ?? workspace.database;
      const title = `New ${kind === 'procedure' ? 'procedure' : kind}`;
      workspace.addTab(await backend.createTemplate(kind, target ?? null), title);
    } catch (e) {
      report(asUiError(e).message);
    }
  }

  /** Opens the table designer in a tab: `table` null for a new table. */
  function openDesigner(table: string | null, schema?: string) {
    workspace.openDesign({ kind: 'table', schema: schema ?? workspace.selected?.schema ?? workspace.database ?? '', table });
  }

  /** Opens the schema editor in a tab: `schema` null for a new schema. */
  function openSchemaEditor(schema: string | null) {
    workspace.openDesign({ kind: 'schema', schema });
  }

  /** Opens the designer tab for tables and schemas, and a script for everything else. */
  function alterObject(target: TreeTarget) {
    if (target.kind === 'schema' || target.name === null) openSchemaEditor(target.schema);
    else if (target.kind === 'table') openDesigner(target.name, target.schema);
    else void alterAsScript(target);
  }

  async function alterAsScript(target: TreeTarget) {
    if (target.kind === 'schema' || target.name === null) return;
    try {
      workspace.addTab(await backend.alterScript(info.id, target.kind, target.schema, target.name), `Alter ${target.name}`);
    } catch (e) {
      report(asUiError(e).message);
    }
  }

  /** Runs what a designer tab has shown for review. The designer reports the outcome itself. */
  async function applyDesign(sql: string, schema: string | null): Promise<string | null> {
    const error = await workspace.execute(sql);
    if (error === null) {
      if (schema) void workspace.loadCatalog(schema, true);
      workspace.treeVersion++;
      historyKey++;
    }
    return error;
  }

  /** Runs a confirmed drop, truncate or rename. */
  async function applyDanger(sql: string): Promise<string | null> {
    const error = await workspace.execute(sql);
    if (error === null) {
      const done = danger;
      danger = null;
      if (done) {
        void workspace.loadCatalog(done.schema, true);
        // A designer left open on something that is gone, or renamed, would only mislead.
        for (const open of [...workspace.tabs]) {
          const design = open.design;
          if (!design || done.action === 'truncate') continue;
          const gone =
            done.action === 'dropSchema'
              ? design.schema === done.schema
              : design.kind === 'table' && design.schema === done.schema && design.table === done.name;
          if (gone) workspace.closeTab(open);
        }
      }
      workspace.treeVersion++;
      historyKey++;
      report(`Done: ${sql}`, 'ok');
    }
    return error;
  }

  function refreshSchemas() {
    workspace.treeVersion++;
  }

  // ---------- writing names into the editor ----------

  function insertIntoEditor(text: string) {
    editors[tab.id]?.insert(text);
  }

  /** A table's name as a query needs it: with its schema unless that is the default. */
  function qualified(schema: string, name: string): string {
    return schema === workspace.database ? quoteIdentifier(name) : `${quoteIdentifier(schema)}.${quoteIdentifier(name)}`;
  }

  /** Writes a SELECT that names every column, ready to trim down. */
  async function insertSelect(schema: string, table: string) {
    try {
      const columns = await backend.tableColumns(info.id, schema, table);
      const list = columns.length > 0 ? columns.map((c) => `  ${quoteIdentifier(c.name)}`).join(',\n') : '  *';
      const current = editors[tab.id]?.getText() ?? '';
      const lead = current.trim() === '' || current.endsWith('\n') ? '' : '\n';
      insertIntoEditor(`${lead}SELECT\n${list}\nFROM ${qualified(schema, table)}\nLIMIT 1000;\n`);
    } catch (e) {
      report(asUiError(e).message);
    }
  }

  function treeMenu(target: TreeTarget | null): MenuItem[] {
    const always: MenuItem[] = [
      { label: 'Create Schema…', action: () => (openSchemaEditor(null)) },
      { label: 'Refresh All', action: refreshSchemas },
    ];
    if (!target) return always;
    const copyName: MenuItem = {
      label: 'Copy Name',
      action: () => void navigator.clipboard.writeText(target.name ?? target.schema),
    };
    const insertName: MenuItem = {
      label: 'Insert Name into Editor',
      action: () => insertIntoEditor(target.name === null ? quoteIdentifier(target.schema) : qualified(target.schema, target.name)),
    };
    const create = (label: string, kind: ObjectKind): MenuItem => ({
      label,
      action: () => void createFromTemplate(kind, target.schema),
    });
    switch (target.kind) {
      case 'schema':
        return [
          { label: 'Set as Default Schema', checked: workspace.database === target.schema, action: () => void useSchema(target.schema) },
          separator,
          { label: 'Alter Schema…', action: () => alterObject(target) },
          { label: 'Create Table…', action: () => openDesigner(null, target.schema) },
          create('Create View…', 'view'),
          create('Create Stored Procedure…', 'procedure'),
          create('Create Function…', 'function'),
          separator,
          copyName,
          insertName,
          separator,
          { label: 'Drop Schema…', action: () => (danger = { action: 'dropSchema', kind: 'table', schema: target.schema, name: target.schema }) },
          separator,
          ...always,
        ];
      case 'table':
      case 'view': {
        const name = target.name ?? '';
        const kind = target.kind;
        const ask = (action: ObjectAction) => () => (danger = { action, kind, schema: target.schema, name });
        return [
          { label: 'Select Rows (Limit 1000)', action: () => openTable(target.schema, name) },
          { label: 'Insert SELECT Statement into Editor', action: () => void insertSelect(target.schema, name) },
          insertName,
          copyName,
          separator,
          ...(kind === 'table'
            ? [
                { label: 'Alter Table…', action: () => alterObject(target) },
                { label: 'Alter Table (SQL Script)…', action: () => void alterAsScript(target) },
                { label: 'Create Table…', action: () => openDesigner(null, target.schema) },
                separator,
                { label: 'Rename Table…', action: ask('rename') },
                { label: 'Truncate Table…', action: ask('truncate') },
                { label: 'Drop Table…', action: ask('drop') },
              ]
            : [
                { label: 'Alter View…', action: () => alterObject(target) },
                create('Create View…', 'view'),
                separator,
                { label: 'Drop View…', action: ask('drop') },
              ]),
          separator,
          ...always,
        ];
      }
      default: {
        const label = { procedure: 'Stored Procedure', function: 'Function', trigger: 'Trigger', event: 'Event' }[target.kind];
        const kind = target.kind;
        return [
          { label: `Alter ${label}…`, action: () => alterObject(target) },
          create(`Create ${label}…`, target.kind),
          separator,
          { label: `Drop ${label}…`, action: () => (danger = { action: 'drop', kind, schema: target.schema, name: target.name ?? '' }) },
          separator,
          copyName,
          separator,
          ...always,
        ];
      }
    }
  }

  // ---------- the connection's menu ----------

  // There is no menu bar: it took a row, and nearly everything in it has a
  // button, a shortcut or a right-click menu of its own. What does not is
  // gathered here, behind one button in the row of connection tabs.
  function showMainMenu(e: MouseEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    contextMenu = {
      items: [
        { label: 'History…', action: () => (historyOpen = true) },
        { label: 'Connection Info', action: () => (infoOpen = true) },
        separator,
        { label: 'New Query Tab', shortcut: keys('Mod+T'), action: () => workspace.addTab() },
        { label: 'Open SQL Script…', shortcut: keys('Mod+O'), action: () => void openScript() },
        { label: 'Find and Replace…', shortcut: keys('Mod+F'), disabled: !!tab.design, action: () => editors[tab.id]?.openSearch() },
        separator,
        { label: 'Create Schema…', action: () => openSchemaEditor(null) },
        { label: 'Create Table…', action: () => openDesigner(null) },
        { label: 'Create View…', action: () => void createFromTemplate('view') },
        { label: 'Create Stored Procedure…', action: () => void createFromTemplate('procedure') },
        { label: 'Create Function…', action: () => void createFromTemplate('function') },
        { label: 'Refresh All Schemas', action: refreshSchemas },
        separator,
        ...adminQueries.map((item): MenuItem => ({ label: item.title, action: () => void openAndRun(item.title, item.sql) })),
        separator,
        { label: 'Settings…', action: () => (app.showSettings = true) },
        { label: 'Keyboard Shortcuts and Help', action: () => (app.showHelp = true) },
        separator,
        { label: 'Close Connection', action: () => void app.close(workspace) },
      ],
      x: rect.right - 250,
      y: rect.bottom + 4,
    };
  }

  // ---------- keyboard ----------

  function onWindowKey(e: KeyboardEvent) {
    if (app.active !== info.id || !(e.ctrlKey || e.metaKey) || document.querySelector('dialog[open]')) return;
    // The editor's shortcuts are for the editor; with the graph in front they would act on something out of sight.
    if (graphShown) return;
    const key = e.key.toLowerCase();
    if (key === 't' && !e.shiftKey && !e.altKey) workspace.addTab();
    else if (key === 'o' && !e.shiftKey && !e.altKey) void openScript();
    else if (key === 'o' && e.shiftKey && !e.altKey) void toggleAutoHide();
    else if (key === 's' && !e.altKey) void saveScript(tab, e.shiftKey);
    else if (key === 'w' && !e.shiftKey && !e.altKey) requestClose(tab);
    else if (key === 'b' && !e.shiftKey && !e.altKey) reformat();
    else if ((key === 'x' || e.code === 'KeyX') && e.altKey) void run(tab, 'explain');
    else return;
    e.preventDefault();
  }

  function insertFromHistory(sql: string) {
    historyOpen = false;
    // Into the query tab; a designer in front has no editor.
    const target = queryTab ?? workspace.addTab();
    workspace.activeTabId = target.id;
    queueMicrotask(() => {
      const editor = editors[target.id];
      editor?.insert(editor.getText().trim() === '' ? sql : `\n${sql}`);
    });
  }

  // ---------- pane sizes ----------

  /** Drags a pane edge. `apply` gets how far the pointer has moved from where it started. */
  function drag(e: PointerEvent, axis: 'x' | 'y', apply: (delta: number) => void) {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const start = axis === 'x' ? e.clientX : e.clientY;
    const move = (m: PointerEvent) => apply((axis === 'x' ? m.clientX : m.clientY) - start);
    const stop = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', stop);
      saveLayout();
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', stop);
  }

  const clampNav = (width: number) => Math.min(Math.max(width, 180), Math.max(180, window.innerWidth - 420));
  const clampInfo = (height: number) => Math.min(Math.max(height, 70), Math.max(70, (aside?.clientHeight ?? 600) - 120));

  function startNavResize(e: PointerEvent) {
    const from = layout.navWidth;
    drag(e, 'x', (delta) => (layout.navWidth = clampNav(from + delta)));
  }

  /** The edge between the schema tree and the object info: dragging up gives the info more room. */
  function startInfoResize(e: PointerEvent) {
    const from = layout.infoHeight;
    drag(e, 'y', (delta) => (layout.infoHeight = clampInfo(from - delta)));
  }

  function paneKey(e: KeyboardEvent, which: 'nav' | 'info') {
    const step = e.key === 'ArrowLeft' || e.key === 'ArrowDown' ? -24 : e.key === 'ArrowRight' || e.key === 'ArrowUp' ? 24 : 0;
    if (step === 0) return;
    if (which === 'nav' && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) layout.navWidth = clampNav(layout.navWidth + step);
    else if (which === 'info' && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) layout.infoHeight = clampInfo(layout.infoHeight + step);
    else return;
    saveLayout();
    e.preventDefault();
  }

  function startResize(e: PointerEvent) {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startHeight = editorHeight;
    const move = (m: PointerEvent) => {
      const max = main.clientHeight - 200;
      editorHeight = Math.min(Math.max(90, startHeight + m.clientY - startY), Math.max(90, max));
    };
    const stop = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', stop);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', stop);
  }

  function modal(node: HTMLDialogElement) {
    node.showModal();
  }

  function resizeByKey(e: KeyboardEvent) {
    if (e.key === 'ArrowUp') editorHeight = Math.max(90, editorHeight - 24);
    else if (e.key === 'ArrowDown') editorHeight = Math.min(main.clientHeight - 200, editorHeight + 24);
    else return;
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onWindowKey} />

<div class="workspace">
  <!-- Shown at the end of the row of connection tabs while this connection is in front. -->
  <div class="row-tools" use:placeIn={app.toolsHost} hidden={app.active !== info.id}>
    <button
      class="quiet graph-button"
      aria-pressed={graphShown}
      disabled={!graphShown && !graphResult}
      title={graphShown
        ? 'Back to the query and its results'
        : graphResult
          ? 'Draw a graph from the result in front'
          : 'A graph is drawn from a finished result: run a query that returns rows first'}
      onclick={toggleGraph}>Graph</button
    >
    <button class="quiet menu-button" aria-label="Menu" aria-haspopup="menu" title="History, connection info, create, server information, settings, help" onclick={showMainMenu}>&#9776;</button>
  </div>

  {#if graphShown && tab.graph}
    {@const charted = tab.results.find((r) => r.id === tab.graph?.resultId)}
    <GraphView
      graph={tab.graph}
      title={`${tab.title}${charted ? ` \u00b7 ${charted.title} \u00b7 ${formatCount(charted.rows)} rows` : ''}`}
      onclose={closeGraph}
    />
  {/if}

  <div class="columns" hidden={graphShown}>
    <aside bind:this={aside} hidden={!navigatorShown} style:width={`${layout.navWidth}px`} aria-label="Navigator">
      <div class="side-body">
        <SchemaTree
          workspaceId={info.id}
          current={workspace.database}
          version={workspace.treeVersion}
          selected={workspace.selected}
          onselect={(target) => (workspace.selected = target)}
          onopen={openTable}
          onuse={useSchema}
          oncontext={(target, x, y) => (contextMenu = { items: treeMenu(target), x, y })}
          onloaded={(schema) => void workspace.loadCatalog(schema)}
          oncreate={(kind, schema) => (kind === 'table' ? openDesigner(null, schema) : void createFromTemplate(kind, schema))}
        />
      </div>
      <!-- A focusable separator is an interactive widget (it resizes with the arrow keys). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="splitter"
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize the object info"
        aria-valuenow={layout.infoHeight}
        tabindex="0"
        onpointerdown={startInfoResize}
        onkeydown={(e) => paneKey(e, 'info')}
      ></div>
      <div class="info-pane" style:height={`${layout.infoHeight}px`}>
        <InfoPane {workspace} oninsert={(name) => insertIntoEditor(quoteIdentifier(name))} />
      </div>
    </aside>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="splitter upright"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the navigator"
      aria-valuenow={layout.navWidth}
      tabindex="0"
      hidden={!navigatorShown}
      onpointerdown={startNavResize}
      onkeydown={(e) => paneKey(e, 'nav')}
    ></div>

    <div class="main" bind:this={main}>
      <div class="editor-tabs">
        <div class="tab-strip" role="tablist" aria-label="Query and designer tabs" bind:this={strip}>
          {#each workspace.tabs as t (t.id)}
            <div class="editor-tab" class:active={t.id === workspace.activeTabId} class:running={t.running}>
              <button
                role="tab"
                aria-selected={t.id === workspace.activeTabId}
                title={t.running
                  ? `${t.title}: a query is running`
                  : t.design
                    ? t.designDirty
                      ? `${t.title}: changes not applied`
                      : t.title
                    : (t.filePath ?? t.title)}
                onclick={() => (workspace.activeTabId = t.id)}
              >
                {#if t.running}<span class="pulse" aria-hidden="true"></span>{/if}{tabLabel(t)}
              </button>
              <button class="quiet close" aria-label={`Close ${t.title}`} title={`Close tab (${keys('Mod+W')})`} onclick={() => requestClose(t)}>&#215;</button>
            </div>
          {/each}
          <button class="quiet add" aria-label="New query tab" title={`New query tab (${keys('Mod+T')})`} onclick={() => workspace.addTab()}>+</button>
        </div>
        <div class="tab-nav">
          <button class="quiet" aria-label="First tab" title="First tab" disabled={workspace.tabs[0]?.id === workspace.activeTabId} onclick={() => stepTab('first')}>&laquo;</button>
          <button class="quiet" aria-label="Previous tab" title="Previous tab" disabled={workspace.tabs[0]?.id === workspace.activeTabId} onclick={() => stepTab(-1)}>&lsaquo;</button>
          <button class="quiet" aria-label="Next tab" title="Next tab" disabled={workspace.tabs.at(-1)?.id === workspace.activeTabId} onclick={() => stepTab(1)}>&rsaquo;</button>
          <button class="quiet" aria-label="Last tab" title="Last tab" disabled={workspace.tabs.at(-1)?.id === workspace.activeTabId} onclick={() => stepTab('last')}>&raquo;</button>
          <button class="quiet" aria-label="All open tabs" aria-haspopup="menu" title={`All open tabs (${workspace.tabs.length})`} onclick={showTabList}>&#9662;</button>
        </div>
      </div>

      <div class="toolbar" role="toolbar" aria-label="Editor" hidden={!!tab.design}>
        <button onclick={() => workspace.addTab()} title={`A new, empty query tab (${keys('Mod+T')})`}>New</button>
        <button onclick={openScript} title={`Open a SQL script file (${keys('Mod+O')})`}>Open</button>
        <button onclick={() => saveScript(tab)} title={`Save this script (${keys('Mod+S')})`}>Save</button>
        <button onclick={() => saveScript(tab, true)} title={`Save this script under another name (${keys('Mod+Shift+S')})`}>Save As</button>
        <span class="divider"></span>
        <button class="primary" disabled={tab.running} title={`Execute the selection, or everything (${keys('Mod+Shift+Enter')})`} onclick={() => run(tab, 'script')}>
          &#9654; Execute
        </button>
        <button
          disabled={tab.running}
          aria-label="Execute statement"
          title={`Execute Statement: run the statement under the cursor (${keys('Mod+Enter')})`}
          onclick={() => run(tab, 'statement')}
        >
          ES
        </button>
        <button disabled={tab.running} title={`Show the execution plan of the statement under the cursor (${keys('Mod+Alt+X')})`} onclick={() => run(tab, 'explain')}>
          Explain
        </button>
        <button
          disabled={tab.running}
          title={`Run a long query as many short ones, one for each slice of a time range, so that none times out. The query marks its range with {start} and {end}.`}
          onclick={sliceQuery}>In Slices&hellip;</button
        >
        <button class="danger" disabled={!tab.running} title="Stop the running statement" onclick={() => workspace.cancel(tab)}>&#9632; Stop</button>
        <span class="divider"></span>
        <label class="limit">
          <span class="muted">Limit to</span>
          <select bind:value={workspace.maxRows} aria-label="Row limit">
            {#each rowLimits as option}
              <option value={option.value}>{option.label}</option>
            {/each}
          </select>
        </label>
        <span class="divider"></span>
        <button onclick={reformat} title={`Reformat the selection, or the whole script (${keys('Mod+B')})`}>Reformat</button>
        <span class="divider"></span>
        <button
          class="pane-toggle"
          aria-pressed={workspace.showNavigator}
          title="Show or hide the navigator (schemas and object info)"
          onclick={() => (workspace.showNavigator = !workspace.showNavigator)}>Navigator</button
        >
        <button
          class="pane-toggle"
          class:auto={autoHide}
          aria-pressed={workspace.showOutput}
          title={`Show or hide the output (messages and results). Double-click, or press ${keys('Mod+Shift+O')}, to have it hide itself while you edit: that is ${autoHide ? 'on' : 'off'} now.`}
          onclick={(e) => {
            // The second click of a double-click. The first has already been acted on; switching puts the output back in view.
            if (e.detail === 2) void toggleAutoHide();
            else if (tucked && workspace.showOutput) setOutput('open');
            else workspace.showOutput = !workspace.showOutput;
          }}
          >Output</button
        >
      </div>

      {#each workspace.tabs as t (t.id)}
        {#if t.design}
          <!-- A designer has the whole area: it has no results of its own. -->
          <div class="design-area" hidden={t.id !== workspace.activeTabId}>
            {#if t.design.kind === 'table'}
              <TableDesigner {workspace} tab={t} onapply={applyDesign} />
            {:else}
              <SchemaEditor {workspace} tab={t} onapply={applyDesign} />
            {/if}
          </div>
        {:else}
          {@const split = workspace.showOutput && hasOutput(t) && !tucked}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="editor-area"
            class:fill={!split}
            style:height={split ? `${editorHeight}px` : undefined}
            hidden={t.id !== workspace.activeTabId}
            onpointerdown={() => setOutput('tucked')}
          >
            <Editor
              bind:this={editors[t.id]}
              value={t.text}
              {mariadb}
              catalog={workspace.catalog}
              defaultSchema={workspace.database}
              onchange={(text) => (t.text = text)}
              onrun={(all) => run(t, all ? 'script' : 'statement')}
            />
          </div>
        {/if}
      {/each}

      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="splitter"
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize editor"
        aria-valuenow={editorHeight}
        tabindex="0"
        hidden={!workspace.showOutput || !!tab.design || !hasOutput(tab) || tucked}
        onpointerdown={(e) => {
          setOutput('open');
          startResize(e);
        }}
        onpointerenter={() => clearTimeout(outputTimer)}
        onpointerleave={leaveOutput}
        onkeydown={resizeByKey}
      ></div>

      {#each workspace.tabs as t (t.id)}
        {#if !t.design}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="results-area"
            hidden={!workspace.showOutput || !hasOutput(t) || t.id !== workspace.activeTabId || tucked}
            onpointerdown={() => setOutput('open')}
            onfocusin={() => setOutput('open')}
            onpointerenter={() => clearTimeout(outputTimer)}
            onpointerleave={leaveOutput}
          >
            <Results {workspace} tab={t} />
          </div>
        {/if}
      {/each}

      {#if tucked && workspace.showOutput && !tab.design && hasOutput(tab)}
        <!-- The output is put away: the pointer resting here, or a click, brings it back. -->
        <button
          class="output-edge"
          title="Output is hidden while editing. Rest the pointer here to look at it; click to keep it open."
          aria-label="Show the output"
          onpointerenter={() => setOutput('peek', 140)}
          onpointerleave={() => clearTimeout(outputTimer)}
          onclick={() => setOutput('open')}
        >
          <span aria-hidden="true"></span>
        </button>
      {/if}
    </div>
  </div>

  <footer class="status">
    <span title={info.info.version.raw}>{serverLabel(info.info.version)}</span>
    <span>{info.user}@{info.host}</span>
    <span class={`security ${security.level}`}>{security.text}</span>
    <span>Schema: {workspace.database ?? 'none selected'}</span>
    {#if info.readOnly}<span class="badge" title="Only reads are allowed on this connection">Read-only</span>{/if}
    {#if info.production}<span class="badge production" title="Risky statements ask for confirmation">Production</span>{/if}
    <span class="spacer"></span>
    {#if tab.slices}
      {@const s = tab.slices}
      <!-- A run in time slices: how many are in, and how long the rest should take. -->
      <span
        class="slices"
        role="progressbar"
        aria-label="Time slices done"
        aria-valuemin="0"
        aria-valuemax={s.total}
        aria-valuenow={s.done}
        title={`Running in time slices${s.threads > 1 ? `, ${s.threads} at a time` : ''}: ${s.done} of ${s.total} done`}
      >
        <span class="slice-mark" class:fast={s.threads > 1} aria-hidden="true">{s.threads > 1 ? '\u21C9' : '\u25A6'}</span>
        <span class="slice-bar"><span style:width={`${(s.done / Math.max(1, s.total)) * 100}%`}></span></span>
        <span>{s.done}/{s.total}{s.threads > 1 ? ` \u00d7${s.threads}` : ''}</span>
        {#if s.etaMs !== null}<span class="muted">{timeLeft(s.etaMs)}</span>{/if}
      </span>
    {/if}
    <!-- The result in front puts its notices, selection totals, time, state and row count here. -->
    <span class="result-status" bind:this={workspace.statusHost}></span>
    {#if tab.design || tab.activeResult === 'messages' || !workspace.showOutput}
      <!-- No result in front: say whether something is running, or how long the last run took. -->
      {#if tab.running}
        <span>Running&hellip;</span>
      {:else if tab.lastRunMs !== null}
        <span class="muted">Last run {formatDuration(tab.lastRunMs)}</span>
      {/if}
    {/if}
  </footer>
</div>

{#if contextMenu}
  <Menu items={contextMenu.items} label="Schema actions" x={contextMenu.x} y={contextMenu.y} onclose={() => (contextMenu = null)} />
{/if}

{#if danger}
  <ConfirmSqlDialog {...danger} onclose={() => (danger = null)} onapply={applyDanger} />
{/if}

{#if confirmation}
  <dialog use:modal aria-labelledby="confirm-run-title" oncancel={() => answerConfirmation(false)}>
    <div class="prompt confirm">
      <h2 id="confirm-run-title">Run this on {info.name}?</h2>
      <p class="warning-box">This connection is marked as production. Check these statements before they run.</p>
      <ul>
        {#each confirmation.concerns as concern (concern.index)}
          <li>
            <strong>{concern.reason}</strong>
            <code>{concern.sql}</code>
          </li>
        {/each}
      </ul>
      <div class="buttons">
        <button onclick={() => answerConfirmation(false)}>Cancel</button>
        <button class="danger" onclick={() => answerConfirmation(true)}>Run Anyway</button>
      </div>
    </div>
  </dialog>
{/if}

<dialog bind:this={closeDialog} aria-labelledby="close-title" onclose={() => (closing = null)}>
  <div class="prompt">
    {#if closing?.design}
      <h2 id="close-title">Close {closing.title} without applying?</h2>
      <p class="muted">The changes made in this tab have not been applied. Closing it discards them; nothing on the server changes.</p>
      <div class="buttons">
        <button onclick={() => closeDialog.close()}>Keep Editing</button>
        <button class="danger" onclick={() => finishClose(false)}>Discard Changes</button>
      </div>
    {:else}
      <h2 id="close-title">Save changes to {closing?.title}?</h2>
      <p class="muted">Your changes will be lost if you do not save them.</p>
      <div class="buttons">
        <button onclick={() => closeDialog.close()}>Cancel</button>
        <button class="danger" onclick={() => finishClose(false)}>Don't Save</button>
        <button class="primary" onclick={() => finishClose(true)}>Save</button>
      </div>
    {/if}
  </div>
</dialog>

{#if slicing}
  <SliceDialog sql={slicing.sql} last={slicing.tab.sliceSetup} onrun={runSlices} onclose={() => (slicing = null)} />
{/if}

{#if historyOpen}
  <dialog use:modal class="panel-dialog" aria-labelledby="history-title" onclose={() => (historyOpen = false)}>
    <div class="panel-head">
      <h2 id="history-title">History</h2>
      <span class="muted">Double-click a statement, or use its button, to put it in the editor.</span>
      <span class="spacer"></span>
      <button onclick={() => (historyOpen = false)}>Close</button>
    </div>
    <div class="panel-body">
      <HistoryPanel refreshKey={historyKey} oninsert={insertFromHistory} />
    </div>
  </dialog>
{/if}

{#if infoOpen}
  <dialog use:modal aria-labelledby="info-title" onclose={() => (infoOpen = false)}>
    <div class="prompt about">
      <h2 id="info-title">{info.name}</h2>
      <table>
        <tbody>
          <tr><td>Host</td><td>{info.host}</td></tr>
          <tr><td>Logged in as</td><td>{info.info.currentUser}</td></tr>
          <tr><td>Server</td><td>{serverLabel(info.info.version)}</td></tr>
          <tr><td>Version</td><td>{info.info.version.raw}</td></tr>
          {#if info.info.versionComment}<tr><td>Edition</td><td>{info.info.versionComment}</td></tr>{/if}
          <tr><td>Connection id</td><td>{info.info.connectionId}</td></tr>
          <tr><td>Encryption</td><td>{tls ?? 'None'}</td></tr>
          {#if info.info.tlsCipher}<tr><td>Cipher</td><td>{info.info.tlsCipher}</td></tr>{/if}
          <tr><td>Verification</td><td>{security.text}</td></tr>
          <tr><td>Default schema</td><td>{workspace.database ?? 'none selected'}</td></tr>
          <tr><td>Read-only</td><td>{info.readOnly ? 'Yes' : 'No'}</td></tr>
          <tr><td>Marked as production</td><td>{info.production ? 'Yes' : 'No'}</td></tr>
        </tbody>
      </table>
      <div class="buttons">
        <button class="primary" onclick={() => (infoOpen = false)}>Close</button>
      </div>
    </div>
  </dialog>
{/if}

<style>
  .workspace {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .row-tools {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .row-tools button {
    padding: 2px 9px;
  }

  /* Navigator and Output, in the editor's toolbar. */
  .toolbar .pane-toggle {
    color: var(--muted);
    border-color: transparent;
    background: transparent;
  }

  .toolbar .pane-toggle[aria-pressed='true'] {
    background: var(--panel-2);
    border-color: var(--border);
    color: var(--text);
  }

  /* The Output button takes the theme's colour while the output hides itself
     during editing. A colour and not a mark beside the word, so the button
     stays the width it was and the row of buttons does not wrap. */
  .toolbar .pane-toggle.auto {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
    color: color-mix(in srgb, var(--accent) 55%, var(--muted));
  }

  /* A tint of the colour, not the colour itself: it is a state to notice, not a call to press. */
  .toolbar .pane-toggle.auto[aria-pressed='true'] {
    background: color-mix(in srgb, var(--accent) 14%, var(--panel-2));
    color: var(--text);
  }

  .slices {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-variant-numeric: tabular-nums;
  }

  .slice-mark {
    color: var(--accent);
    font-size: 13px;
    line-height: 1;
  }

  .slice-mark.fast {
    animation: slice-pulse 1.1s ease-in-out infinite;
  }

  @keyframes slice-pulse {
    50% {
      opacity: 0.35;
    }
  }

  .slice-bar {
    width: 84px;
    height: 5px;
    border-radius: 3px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    overflow: hidden;
  }

  .slice-bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s linear;
  }

  .row-tools .graph-button[aria-pressed='true'] {
    background: var(--panel-2);
    border-color: var(--border);
  }

  .row-tools .menu-button {
    font-size: 15px;
    line-height: 18px;
    color: var(--text);
  }

  .columns {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  aside {
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--panel);
  }

  .side-body {
    flex: 1;
    min-height: 60px;
  }

  .info-pane {
    flex: none;
    min-height: 0;
  }

  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .editor-tabs {
    flex: none;
    display: flex;
    align-items: stretch;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }

  .tab-strip {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 4px 8px 0;
    /* One row, no scroll bar. Tabs that do not fit are reached with the
       arrows and the list at the end of the row. */
    overflow: hidden;
  }

  .editor-tab {
    flex: none;
    display: flex;
    align-items: center;
    border: 1px solid transparent;
    border-top-width: 2px;
    border-bottom: none;
    border-radius: 5px 5px 0 0;
  }

  /* A tab with a query running stands out wherever it is in the row. */
  .editor-tab.running {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }

  .pulse {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-right: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.1s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.25;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }

  .tab-nav {
    flex: none;
    display: flex;
    align-items: center;
    padding: 0 4px;
    border-left: 1px solid var(--border);
  }

  .tab-nav button {
    min-width: 26px;
    padding: 0 6px;
    font-size: 22px;
    font-weight: 600;
    line-height: 26px;
    color: var(--text);
  }

  /* The triangle is a small glyph to begin with. */
  .tab-nav button:last-child {
    font-size: 18px;
  }

  .editor-tab > button:first-child {
    border: none;
    background: transparent;
    padding: 4px 4px 4px 10px;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .editor-tab.active {
    background: var(--bg);
    border-color: var(--border);
    border-top-color: var(--accent);
    font-weight: 600;
  }

  .close,
  .add {
    padding: 0 7px;
    font-size: 15px;
    line-height: 20px;
    color: var(--muted);
  }

  .add {
    flex: none;
    align-self: center;
  }

  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    /* No scroll bar: buttons that do not fit go onto another row. */
    flex-wrap: wrap;
  }

  .toolbar button {
    padding: 3px 9px;
  }

  .divider {
    flex: none;
    width: 1px;
    align-self: stretch;
    margin: 2px 4px;
    background: var(--border);
  }

  .limit {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }

  .limit select {
    padding: 3px 6px;
  }

  .spacer {
    flex: 1;
  }

  .editor-area {
    flex: none;
    min-height: 0;
  }

  .design-area {
    flex: 1;
    min-height: 0;
  }

  .editor-area.fill {
    flex: 1;
  }

  .splitter {
    flex: none;
    height: 6px;
    cursor: row-resize;
    background: var(--panel);
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    touch-action: none;
  }

  .splitter:hover,
  .splitter:focus-visible {
    background: var(--accent);
  }

  /* The edge between the navigator and the editor. */
  .splitter.upright {
    width: 6px;
    height: auto;
    cursor: col-resize;
    border-top: none;
    border-bottom: none;
    border-left: 1px solid var(--border);
    border-right: 1px solid var(--border);
  }

  dialog.panel-dialog {
    width: min(760px, calc(100vw - 32px));
  }

  .panel-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 14px 16px 6px;
  }

  .panel-head h2 {
    margin: 0;
    font-size: 16px;
  }

  .panel-body {
    height: min(60vh, 560px);
    border-top: 1px solid var(--border);
  }

  .results-area {
    flex: 1;
    min-height: 0;
  }

  /* Where the output went: a thin edge along the bottom with a grip in it. */
  .output-edge {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 9px;
    padding: 0;
    border: none;
    border-top: 1px solid var(--border);
    border-radius: 0;
    background: var(--panel);
    cursor: n-resize;
  }

  .output-edge span {
    width: 44px;
    height: 3px;
    border-radius: 2px;
    background: var(--border);
  }

  .output-edge:hover,
  .output-edge:focus-visible {
    background: var(--panel-2);
  }

  .output-edge:hover span,
  .output-edge:focus-visible span {
    background: var(--accent);
  }

  .status {
    flex: none;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 3px 10px;
    border-top: 1px solid var(--border);
    background: var(--panel-2);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }

  .status > span {
    /* When the row is too narrow, the connection details give way well before the result's status does. */
    flex-shrink: 6;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status > .result-status {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
  }

  .status > .result-status:empty {
    display: none;
  }

  .security.good {
    color: var(--ok);
  }

  .security.caution {
    color: var(--warn-text);
    background: var(--warn-bg);
    border-radius: 3px;
    padding: 0 5px;
  }

  .security.bad {
    color: var(--danger);
    font-weight: 600;
  }

  .badge {
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 5px;
  }

  .badge.production {
    color: var(--danger);
    border-color: var(--danger);
    font-weight: 600;
  }

  .prompt.confirm {
    width: 560px;
  }

  .prompt ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 40vh;
    overflow: auto;
  }

  .prompt li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }

  .prompt code {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .prompt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 20px;
    width: 420px;
    max-width: 100%;
  }

  .prompt.about {
    width: 480px;
  }

  .prompt h2 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }

  .prompt p {
    margin: 0;
  }

  .prompt table {
    border-collapse: collapse;
  }

  .prompt td {
    padding: 3px 0;
    border-bottom: 1px solid var(--border);
  }

  .prompt td:first-child {
    color: var(--muted);
    white-space: nowrap;
  }

  .prompt td:last-child {
    text-align: right;
    padding-left: 16px;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
