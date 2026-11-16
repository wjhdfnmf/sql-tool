<script lang="ts">
  import type { EditorTab, WorkspaceState } from '../lib/state.svelte';
  import { formatDuration, quoteIdentifier, serverLabel } from '../lib/types';
  import Editor from './Editor.svelte';
  import HistoryPanel from './HistoryPanel.svelte';
  import Results from './Results.svelte';
  import SchemaTree from './SchemaTree.svelte';

  interface Props {
    workspace: WorkspaceState;
  }

  let { workspace }: Props = $props();

  let editors = $state<Record<string, Editor | undefined>>({});
  let sidebar = $state<'schemas' | 'history'>('schemas');
  let editorHeight = $state(260);
  let historyKey = $state(0);
  let main: HTMLDivElement;

  const info = $derived(workspace.info);
  const tab = $derived(workspace.activeTab);
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

  async function run(target: EditorTab, all: boolean) {
    const editor = editors[target.id];
    if (!editor) return;
    const selected = editor.getSelection();
    if (!all && selected.trim() !== '') await workspace.run(target, selected, { kind: 'all' });
    else if (all) await workspace.run(target, editor.getText(), { kind: 'all' });
    else await workspace.run(target, editor.getText(), { kind: 'at', offset: editor.getCursor() });
    historyKey++;
  }

  async function openTable(schema: string, table: string) {
    const sql = `SELECT * FROM ${quoteIdentifier(schema)}.${quoteIdentifier(table)} LIMIT 1000;`;
    const created = workspace.addTab(sql);
    created.title = table;
    await workspace.run(created, sql, { kind: 'all' });
    historyKey++;
  }

  async function useSchema(schema: string) {
    await workspace.run(tab, `USE ${quoteIdentifier(schema)}`, { kind: 'all' });
    historyKey++;
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
      const max = main.clientHeight - 160;
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
    else if (e.key === 'ArrowDown') editorHeight = Math.min(main.clientHeight - 160, editorHeight + 24);
    else return;
    e.preventDefault();
  }
</script>

<div class="workspace">
  <aside>
    <div class="side-tabs" role="tablist" aria-label="Sidebar">
      <button role="tab" aria-selected={sidebar === 'schemas'} class:active={sidebar === 'schemas'} onclick={() => (sidebar = 'schemas')}>
        Schemas
      </button>
      <button role="tab" aria-selected={sidebar === 'history'} class:active={sidebar === 'history'} onclick={() => (sidebar = 'history')}>
        History
      </button>
    </div>
    <div class="side-body" hidden={sidebar !== 'schemas'}>
      <SchemaTree workspaceId={info.id} current={workspace.database} onopen={openTable} onuse={useSchema} />
    </div>
    <div class="side-body" hidden={sidebar !== 'history'}>
      <HistoryPanel refreshKey={historyKey} oninsert={insertFromHistory} />
    </div>
  </aside>

  <div class="main" bind:this={main}>
    <div class="editor-tabs" role="tablist" aria-label="Query tabs">
      {#each workspace.tabs as t (t.id)}
        <div class="editor-tab" class:active={t.id === workspace.activeTabId}>
          <button role="tab" aria-selected={t.id === workspace.activeTabId} onclick={() => (workspace.activeTabId = t.id)}>
            {t.title}{t.running ? ' …' : ''}
          </button>
          <button class="quiet close" aria-label={`Close ${t.title}`} title="Close tab" onclick={() => workspace.closeTab(t)}>&#215;</button>
        </div>
      {/each}
      <button class="quiet add" aria-label="New query tab" title="New query tab" onclick={() => workspace.addTab()}>+</button>
    </div>

    <div class="toolbar">
      <button class="primary" disabled={tab.running} title="Run the statement at the cursor, or the selection (Ctrl+Enter)" onclick={() => run(tab, false)}>
        &#9654; Run
      </button>
      <button disabled={tab.running} title="Run every statement in this tab (Ctrl+Shift+Enter)" onclick={() => run(tab, true)}>Run all</button>
      <button class="danger" disabled={!tab.running} title="Stop the running statement" onclick={() => workspace.cancel(tab)}>&#9632; Stop</button>
      <span class="spacer"></span>
      <label class="limit">
        <span class="muted">Fetch up to</span>
        <select bind:value={workspace.maxRows}>
          {#each rowLimits as option}
            <option value={option.value}>{option.label}</option>
          {/each}
        </select>
      </label>
    </div>

    {#each workspace.tabs as t (t.id)}
      <div class="editor-area" style:height={`${editorHeight}px`} hidden={t.id !== workspace.activeTabId}>
        <Editor
          bind:this={editors[t.id]}
          value={t.text}
          mariadb={info.info.version.flavor === 'mariadb'}
          onchange={(text) => (t.text = text)}
          onrun={(all) => run(t, all)}
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
      onpointerdown={startResize}
      onkeydown={resizeByKey}
    ></div>

    {#each workspace.tabs as t (t.id)}
      <div class="results-area" hidden={t.id !== workspace.activeTabId}>
        <Results tab={t} />
      </div>
    {/each}
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

<style>
  .workspace {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 270px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
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
    padding: 7px;
    color: var(--muted);
  }

  .side-tabs button.active {
    border-bottom-color: var(--accent);
    color: var(--text);
    font-weight: 600;
  }

  .side-body {
    flex: 1;
    min-height: 0;
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
    gap: 2px;
    padding: 4px 8px 0;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
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

  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }

  .limit {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .spacer {
    flex: 1;
  }

  .editor-area {
    flex: none;
    min-height: 0;
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
    grid-column: 1 / -1;
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
</style>
