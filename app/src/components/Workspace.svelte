<script lang="ts">
  import { format } from 'sql-formatter';
  import { adminQueries } from '../lib/admin';
  import { backend } from '../lib/backend';
  import { separator, type MenuDef, type MenuItem } from '../lib/menu';
  import { app, type EditorTab, type WorkspaceState } from '../lib/state.svelte';
  import { asUiError, formatDuration, quoteIdentifier, serverLabel, type ObjectKind, type TreeTarget } from '../lib/types';
  import AdminPanel from './AdminPanel.svelte';
  import Editor from './Editor.svelte';
  import HistoryPanel from './HistoryPanel.svelte';
  import InfoPane from './InfoPane.svelte';
  import Menu from './Menu.svelte';
  import MenuBar from './MenuBar.svelte';
  import Results from './Results.svelte';
  import SchemaDialog from './SchemaDialog.svelte';
  import SchemaTree from './SchemaTree.svelte';

  interface Props {
    workspace: WorkspaceState;
  }

  let { workspace }: Props = $props();

  let editors = $state<Record<string, Editor | undefined>>({});
  let navTab = $state<'schemas' | 'admin' | 'history'>('schemas');
  let editorHeight = $state(260);
  let historyKey = $state(0);
  let main: HTMLDivElement;
  let contextMenu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null);
  /** Open schema dialog: `schema` null creates, a name alters. */
  let schemaDialog = $state<{ schema: string | null } | null>(null);
  let closing = $state<EditorTab | null>(null);
  let closeDialog: HTMLDialogElement;
  let aboutDialog: HTMLDialogElement;

  const info = $derived(workspace.info);
  const tab = $derived(workspace.activeTab);
  const mariadb = $derived(info.info.version.flavor === 'mariadb');
  const tls = $derived(info.info.tlsVersion?.replace('TLSv', 'TLS ') ?? null);
  const security = $derived(
    info.security === 'verified'
      ? { text: `Encrypted (${tls}), server verified`, level: 'good' }
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

  async function openAndRun(title: string, sql: string) {
    const created = workspace.addTab(sql, title);
    await workspace.run(created, sql, { kind: 'all' });
    historyKey++;
  }

  function openTable(schema: string, table: string) {
    void openAndRun(table, `SELECT * FROM ${quoteIdentifier(schema)}.${quoteIdentifier(table)} LIMIT 1000;`);
  }

  async function useSchema(schema: string) {
    // In the active tab's own session, without disturbing its results.
    await workspace.run(tab, `USE ${quoteIdentifier(schema)}`, { kind: 'all' }, true);
    historyKey++;
  }

  // ---------- script files ----------

  async function openScript() {
    try {
      const file = await backend.scriptOpen();
      if (!file) return;
      const created = workspace.addTab(file.text, file.name);
      created.filePath = file.path;
    } catch (e) {
      tab.report(asUiError(e).message);
    }
  }

  async function saveScript(target: EditorTab, saveAs = false): Promise<boolean> {
    const text = editors[target.id]?.getText() ?? target.text;
    try {
      const suggested = target.filePath ? target.title : `${target.title}.sql`;
      const saved = await backend.scriptSave(saveAs ? null : target.filePath, suggested, text);
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
    // Only a file on disk can lose work silently; scratch tabs just close.
    if (target.filePath && target.dirty) {
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
    const selection = editor.getSelection();
    const partial = selection.trim() !== '';
    const source = partial ? selection : editor.getText();
    if (source.trim() === '') return;
    if (/^\s*delimiter\s/im.test(source)) {
      tab.report('A script that uses DELIMITER cannot be reformatted as a whole. Select the part to reformat.');
      return;
    }
    try {
      const formatted = format(source, { language: mariadb ? 'mariadb' : 'mysql', keywordCase: 'upper', tabWidth: 2 });
      if (partial) editor.insert(formatted);
      else editor.replaceAll(formatted + '\n');
    } catch (e) {
      const reason = e instanceof Error ? e.message.split('\n')[0] : String(e);
      tab.report(`Could not reformat: ${reason}`);
    }
  }

  // ---------- schema objects ----------

  async function createFromTemplate(kind: ObjectKind, schema?: string) {
    try {
      const target = schema ?? workspace.selected?.schema ?? workspace.database;
      const title = `New ${kind === 'procedure' ? 'procedure' : kind}`;
      workspace.addTab(await backend.createTemplate(kind, target ?? null), title);
    } catch (e) {
      tab.report(asUiError(e).message);
    }
  }

  async function alterObject(target: TreeTarget) {
    if (target.kind === 'schema' || target.name === null) {
      schemaDialog = { schema: target.schema };
      return;
    }
    try {
      workspace.addTab(await backend.alterScript(info.id, target.kind, target.schema, target.name), `Alter ${target.name}`);
    } catch (e) {
      tab.report(asUiError(e).message);
    }
  }

  async function applySchemaChange(sql: string): Promise<string | null> {
    const error = await workspace.execute(sql);
    if (error === null) {
      schemaDialog = null;
      workspace.treeVersion++;
      historyKey++;
      tab.report(`Done: ${sql}`, 'ok');
    }
    return error;
  }

  function refreshSchemas() {
    workspace.treeVersion++;
  }

  function treeMenu(target: TreeTarget | null): MenuItem[] {
    const always: MenuItem[] = [
      { label: 'Create Schema…', action: () => (schemaDialog = { schema: null }) },
      { label: 'Refresh All', action: refreshSchemas },
    ];
    if (!target) return always;
    const copyName: MenuItem = {
      label: 'Copy Name',
      action: () => void navigator.clipboard.writeText(target.name ?? target.schema),
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
          { label: 'Alter Schema…', action: () => void alterObject(target) },
          create('Create Table…', 'table'),
          create('Create View…', 'view'),
          create('Create Stored Procedure…', 'procedure'),
          create('Create Function…', 'function'),
          separator,
          copyName,
          separator,
          ...always,
        ];
      case 'table':
      case 'view': {
        const name = target.name ?? '';
        return [
          { label: 'Select Rows (Limit 1000)', action: () => openTable(target.schema, name) },
          separator,
          { label: target.kind === 'table' ? 'Alter Table…' : 'Alter View…', action: () => void alterObject(target) },
          create(target.kind === 'table' ? 'Create Table…' : 'Create View…', target.kind),
          separator,
          copyName,
          separator,
          ...always,
        ];
      }
      default: {
        const label = { procedure: 'Stored Procedure', function: 'Function', trigger: 'Trigger', event: 'Event' }[target.kind];
        return [
          { label: `Alter ${label}…`, action: () => void alterObject(target) },
          create(`Create ${label}…`, target.kind),
          separator,
          copyName,
          separator,
          ...always,
        ];
      }
    }
  }

  // ---------- menu bar ----------

  const menus = $derived<MenuDef[]>([
    {
      label: 'File',
      items: [
        { label: 'New Query Tab', shortcut: 'Ctrl+T', action: () => workspace.addTab() },
        { label: 'Open SQL Script…', shortcut: 'Ctrl+O', action: () => void openScript() },
        separator,
        { label: 'Save Script', shortcut: 'Ctrl+S', action: () => void saveScript(tab) },
        { label: 'Save Script As…', shortcut: 'Ctrl+Shift+S', action: () => void saveScript(tab, true) },
        separator,
        { label: 'Close Tab', shortcut: 'Ctrl+W', action: () => requestClose(tab) },
        { label: 'Close Connection', action: () => void app.close(workspace) },
      ],
    },
    {
      label: 'Edit',
      items: [
        { label: 'Undo', shortcut: 'Ctrl+Z', action: () => editors[tab.id]?.undo() },
        { label: 'Redo', shortcut: 'Ctrl+Y', action: () => editors[tab.id]?.redo() },
        separator,
        { label: 'Select All', shortcut: 'Ctrl+A', action: () => editors[tab.id]?.selectAll() },
        { label: 'Find and Replace…', shortcut: 'Ctrl+F', action: () => editors[tab.id]?.openSearch() },
        separator,
        { label: 'Reformat Query', shortcut: 'Ctrl+B', action: reformat },
      ],
    },
    {
      label: 'View',
      items: [
        { label: 'Navigator', checked: workspace.showNavigator, action: () => (workspace.showNavigator = !workspace.showNavigator) },
        { label: 'Output', checked: workspace.showOutput, action: () => (workspace.showOutput = !workspace.showOutput) },
        separator,
        { label: 'Schemas', checked: navTab === 'schemas', action: () => showNav('schemas') },
        { label: 'Administration', checked: navTab === 'admin', action: () => showNav('admin') },
        { label: 'History', checked: navTab === 'history', action: () => showNav('history') },
      ],
    },
    {
      label: 'Query',
      items: [
        { label: 'Execute (All or Selection)', shortcut: 'Ctrl+Shift+Enter', disabled: tab.running, action: () => void run(tab, 'script') },
        { label: 'Execute Current Statement', shortcut: 'Ctrl+Enter', disabled: tab.running, action: () => void run(tab, 'statement') },
        { label: 'Explain Current Statement', shortcut: 'Ctrl+Alt+X', disabled: tab.running, action: () => void run(tab, 'explain') },
        { label: 'Stop', disabled: !tab.running, action: () => void workspace.cancel(tab) },
        separator,
        ...rowLimits.map(
          (option): MenuItem => ({
            label: option.value === null ? "Don't Limit" : `Limit to ${option.label}`,
            checked: workspace.maxRows === option.value,
            action: () => (workspace.maxRows = option.value),
          }),
        ),
      ],
    },
    {
      label: 'Database',
      items: [
        { label: 'Manage Connections', action: () => (app.active = 'home') },
        separator,
        { label: 'Refresh All Schemas', action: refreshSchemas },
        { label: 'Create Schema…', action: () => (schemaDialog = { schema: null }) },
        separator,
        { label: 'Create Table…', action: () => void createFromTemplate('table') },
        { label: 'Create View…', action: () => void createFromTemplate('view') },
        { label: 'Create Stored Procedure…', action: () => void createFromTemplate('procedure') },
        { label: 'Create Function…', action: () => void createFromTemplate('function') },
      ],
    },
    {
      label: 'Server',
      items: adminQueries.map((item): MenuItem => ({ label: item.title, action: () => void openAndRun(item.title, item.sql) })),
    },
    {
      label: 'Help',
      items: [{ label: 'About and Keyboard Shortcuts', action: () => aboutDialog.showModal() }],
    },
  ]);

  function showNav(which: typeof navTab) {
    navTab = which;
    workspace.showNavigator = true;
  }

  // ---------- keyboard ----------

  function onWindowKey(e: KeyboardEvent) {
    if (app.active !== info.id || !(e.ctrlKey || e.metaKey) || document.querySelector('dialog[open]')) return;
    const key = e.key.toLowerCase();
    if (key === 't' && !e.shiftKey && !e.altKey) workspace.addTab();
    else if (key === 'o' && !e.shiftKey && !e.altKey) void openScript();
    else if (key === 's' && !e.altKey) void saveScript(tab, e.shiftKey);
    else if (key === 'w' && !e.shiftKey && !e.altKey) requestClose(tab);
    else if (key === 'b' && !e.shiftKey && !e.altKey) reformat();
    else if (key === 'x' && e.altKey) void run(tab, 'explain');
    else return;
    e.preventDefault();
  }

  function insertFromHistory(sql: string) {
    const editor = editors[tab.id];
    editor?.insert(editor.getText().trim() === '' ? sql : `\n${sql}`);
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

  function resizeByKey(e: KeyboardEvent) {
    if (e.key === 'ArrowUp') editorHeight = Math.max(90, editorHeight - 24);
    else if (e.key === 'ArrowDown') editorHeight = Math.min(main.clientHeight - 200, editorHeight + 24);
    else return;
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onWindowKey} />

<div class="workspace">
  <MenuBar {menus} />

  <div class="toolbar main-toolbar" role="toolbar" aria-label="Main">
    <button onclick={() => workspace.addTab()} title="New query tab (Ctrl+T)">New Tab</button>
    <button onclick={openScript} title="Open a SQL script file (Ctrl+O)">Open Script</button>
    <span class="divider"></span>
    <button onclick={() => (schemaDialog = { schema: null })} title="Create a new schema">Create Schema</button>
    <button onclick={() => createFromTemplate('table')} title="Start a CREATE TABLE script">Create Table</button>
    <button onclick={() => createFromTemplate('view')} title="Start a CREATE VIEW script">Create View</button>
    <button onclick={() => createFromTemplate('procedure')} title="Start a CREATE PROCEDURE script">Create Procedure</button>
  </div>

  <div class="columns" class:no-navigator={!workspace.showNavigator}>
    <aside hidden={!workspace.showNavigator} aria-label="Navigator">
      <div class="side-tabs" role="tablist" aria-label="Navigator">
        <button role="tab" aria-selected={navTab === 'schemas'} class:active={navTab === 'schemas'} onclick={() => (navTab = 'schemas')}>
          Schemas
        </button>
        <button role="tab" aria-selected={navTab === 'admin'} class:active={navTab === 'admin'} onclick={() => (navTab = 'admin')}>
          Administration
        </button>
        <button role="tab" aria-selected={navTab === 'history'} class:active={navTab === 'history'} onclick={() => (navTab = 'history')}>
          History
        </button>
      </div>
      <div class="side-body" hidden={navTab !== 'schemas'}>
        <SchemaTree
          workspaceId={info.id}
          current={workspace.database}
          version={workspace.treeVersion}
          selected={workspace.selected}
          onselect={(target) => (workspace.selected = target)}
          onopen={openTable}
          onuse={useSchema}
          oncontext={(target, x, y) => (contextMenu = { items: treeMenu(target), x, y })}
        />
      </div>
      <div class="side-body" hidden={navTab !== 'admin'}>
        <AdminPanel onrun={openAndRun} />
      </div>
      <div class="side-body" hidden={navTab !== 'history'}>
        <HistoryPanel refreshKey={historyKey} oninsert={insertFromHistory} />
      </div>
      <div class="info-pane">
        <InfoPane {workspace} />
      </div>
    </aside>

    <div class="main" bind:this={main}>
      <div class="editor-tabs">
        <div class="tab-strip" role="tablist" aria-label="Query tabs">
          {#each workspace.tabs as t (t.id)}
            <div class="editor-tab" class:active={t.id === workspace.activeTabId}>
              <button
                role="tab"
                aria-selected={t.id === workspace.activeTabId}
                title={t.filePath ?? t.title}
                onclick={() => (workspace.activeTabId = t.id)}
              >
                {t.title}{t.filePath && t.dirty ? ' •' : ''}{t.running ? ' …' : ''}
              </button>
              <button class="quiet close" aria-label={`Close ${t.title}`} title="Close tab (Ctrl+W)" onclick={() => requestClose(t)}>&#215;</button>
            </div>
          {/each}
          <button class="quiet add" aria-label="New query tab" title="New query tab (Ctrl+T)" onclick={() => workspace.addTab()}>+</button>
        </div>
        <div class="toggles">
          <button
            class="quiet"
            aria-pressed={workspace.showNavigator}
            title="Show or hide the navigator"
            onclick={() => (workspace.showNavigator = !workspace.showNavigator)}>Navigator</button
          >
          <button
            class="quiet"
            aria-pressed={workspace.showOutput}
            title="Show or hide the output"
            onclick={() => (workspace.showOutput = !workspace.showOutput)}>Output</button
          >
        </div>
      </div>

      <div class="toolbar" role="toolbar" aria-label="Editor">
        <button onclick={openScript} title="Open a SQL script file (Ctrl+O)">Open</button>
        <button onclick={() => saveScript(tab)} title="Save this script (Ctrl+S)">Save</button>
        <span class="divider"></span>
        <button class="primary" disabled={tab.running} title="Execute the selection, or everything (Ctrl+Shift+Enter)" onclick={() => run(tab, 'script')}>
          &#9654; Execute
        </button>
        <button disabled={tab.running} title="Execute the statement under the cursor (Ctrl+Enter)" onclick={() => run(tab, 'statement')}>
          Execute Statement
        </button>
        <button disabled={tab.running} title="Show the execution plan of the statement under the cursor (Ctrl+Alt+X)" onclick={() => run(tab, 'explain')}>
          Explain
        </button>
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
        <button onclick={reformat} title="Reformat the selection, or the whole script (Ctrl+B)">Reformat</button>
      </div>

      {#each workspace.tabs as t (t.id)}
        <div
          class="editor-area"
          class:fill={!workspace.showOutput}
          style:height={workspace.showOutput ? `${editorHeight}px` : undefined}
          hidden={t.id !== workspace.activeTabId}
        >
          <Editor
            bind:this={editors[t.id]}
            value={t.text}
            {mariadb}
            onchange={(text) => (t.text = text)}
            onrun={(all) => run(t, all ? 'script' : 'statement')}
          />
        </div>
      {/each}

      <!-- A focusable separator is an interactive widget (it resizes with the arrow keys). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="splitter"
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize editor"
        aria-valuenow={editorHeight}
        tabindex="0"
        hidden={!workspace.showOutput}
        onpointerdown={startResize}
        onkeydown={resizeByKey}
      ></div>

      {#each workspace.tabs as t (t.id)}
        <div class="results-area" hidden={!workspace.showOutput || t.id !== workspace.activeTabId}>
          <Results tab={t} />
        </div>
      {/each}
    </div>
  </div>

  <footer class="status">
    <span title={info.info.version.raw}>{serverLabel(info.info.version)}</span>
    <span>{info.user}@{info.host}</span>
    <span class={`security ${security.level}`}>{security.text}</span>
    <span>Schema: {workspace.database ?? 'none selected'}</span>
    <span class="spacer"></span>
    {#if tab.running}
      <span>Running&hellip;</span>
    {:else if tab.lastRunMs !== null}
      <span class="muted">Last run {formatDuration(tab.lastRunMs)}</span>
    {/if}
  </footer>
</div>

{#if contextMenu}
  <Menu items={contextMenu.items} label="Schema actions" x={contextMenu.x} y={contextMenu.y} onclose={() => (contextMenu = null)} />
{/if}

{#if schemaDialog}
  <SchemaDialog workspaceId={info.id} schema={schemaDialog.schema} onclose={() => (schemaDialog = null)} onapply={applySchemaChange} />
{/if}

<dialog bind:this={closeDialog} aria-labelledby="close-title" onclose={() => (closing = null)}>
  <div class="prompt">
    <h2 id="close-title">Save changes to {closing?.title}?</h2>
    <p class="muted">Your changes will be lost if you do not save them.</p>
    <div class="buttons">
      <button onclick={() => closeDialog.close()}>Cancel</button>
      <button class="danger" onclick={() => finishClose(false)}>Don't Save</button>
      <button class="primary" onclick={() => finishClose(true)}>Save</button>
    </div>
  </div>
</dialog>

<dialog bind:this={aboutDialog} aria-labelledby="about-title">
  <div class="prompt about">
    <h2 id="about-title">SQL Tool {app.info?.version ?? ''}</h2>
    <p class="muted">A free, open-source SQL client for MySQL and MariaDB. Early alpha.</p>
    <table>
      <tbody>
        <tr><td>Execute selection or everything</td><td>Ctrl+Shift+Enter</td></tr>
        <tr><td>Execute statement under the cursor</td><td>Ctrl+Enter</td></tr>
        <tr><td>Explain statement under the cursor</td><td>Ctrl+Alt+X</td></tr>
        <tr><td>Reformat query</td><td>Ctrl+B</td></tr>
        <tr><td>New query tab</td><td>Ctrl+T</td></tr>
        <tr><td>Close tab</td><td>Ctrl+W</td></tr>
        <tr><td>Open script</td><td>Ctrl+O</td></tr>
        <tr><td>Save script, Save as</td><td>Ctrl+S, Ctrl+Shift+S</td></tr>
        <tr><td>Find and replace</td><td>Ctrl+F</td></tr>
        <tr><td>Schema tree: actions menu</td><td>Right-click, or Shift+F10</td></tr>
      </tbody>
    </table>
    <div class="buttons">
      <button class="primary" onclick={() => aboutDialog.close()}>Close</button>
    </div>
  </div>
</dialog>

<style>
  .workspace {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .columns {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 290px minmax(0, 1fr);
  }

  .columns.no-navigator {
    grid-template-columns: minmax(0, 1fr);
  }

  aside {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
    background: var(--panel);
  }

  .side-tabs {
    flex: none;
    display: flex;
    border-bottom: 1px solid var(--border);
  }

  .side-tabs button {
    flex: 1;
    border: none;
    border-radius: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    padding: 6px 4px;
    color: var(--muted);
  }

  .side-tabs button.active {
    border-bottom-color: var(--accent);
    color: var(--text);
    font-weight: 600;
  }

  .side-body {
    flex: 3;
    min-height: 0;
  }

  .info-pane {
    flex: 2;
    min-height: 120px;
  }

  .main {
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
    overflow-x: auto;
  }

  .editor-tab {
    display: flex;
    align-items: center;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 5px 5px 0 0;
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
    margin-bottom: -1px;
    padding-bottom: 1px;
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
    align-self: center;
  }

  .toggles {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px;
  }

  .toggles button {
    padding: 2px 8px;
    font-size: 12px;
    color: var(--muted);
  }

  .toggles button[aria-pressed='true'] {
    background: var(--panel-2);
    color: var(--text);
  }

  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }

  .main-toolbar {
    background: var(--panel);
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

  .results-area {
    flex: 1;
    min-height: 0;
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
    overflow: hidden;
    text-overflow: ellipsis;
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

  .prompt td:last-child {
    text-align: right;
    font-family: var(--mono);
    font-size: 12px;
    white-space: nowrap;
    padding-left: 16px;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
