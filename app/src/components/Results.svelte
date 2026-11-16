<script lang="ts">
  import { backend } from '../lib/backend';
  import type { EditorTab, ResultTab } from '../lib/state.svelte';
  import { asUiError, formatCount, formatDuration, type CellValue } from '../lib/types';
  import Grid from './Grid.svelte';

  interface Props {
    tab: EditorTab;
  }

  let { tab }: Props = $props();

  interface OpenCell {
    column: string;
    row: number;
    value: CellValue;
    pretty: string | null;
  }

  let viewer: HTMLDialogElement | undefined = $state();
  let openCell = $state<OpenCell | null>(null);
  let selection = $state<Record<number, string>>({});

  async function showCell(result: ResultTab, row: number, column: number) {
    try {
      const value = await backend.getCell(result.id, row, column);
      let pretty: string | null = null;
      if (value.text !== null && result.columns[column].kind === 'json' && !value.truncated) {
        try {
          pretty = JSON.stringify(JSON.parse(value.text), null, 2);
        } catch {
          // Not valid JSON after all; show it as it is.
        }
      }
      openCell = { column: result.columns[column].name, row, value, pretty };
      viewer?.showModal();
    } catch (e) {
      tab.messages.push({ index: tab.messages.length, sql: '', state: 'error', text: asUiError(e).message, elapsedMs: null });
    }
  }

  async function copyOpenCell() {
    if (openCell?.value.text != null) await navigator.clipboard.writeText(openCell.pretty ?? openCell.value.text);
  }
</script>

<section class="results">
  <div class="tabs" role="tablist" aria-label="Results">
    <button
      role="tab"
      aria-selected={tab.activeResult === 'messages'}
      class:active={tab.activeResult === 'messages'}
      onclick={() => (tab.activeResult = 'messages')}
    >
      Messages
      {#if tab.messages.some((m) => m.state === 'error')}<span class="badge error" aria-label="has errors"></span>{/if}
    </button>
    {#each tab.results as result (result.id)}
      <button
        role="tab"
        aria-selected={tab.activeResult === result.id}
        class:active={tab.activeResult === result.id}
        onclick={() => (tab.activeResult = result.id)}
      >
        {result.title}
        <span class="muted">{formatCount(result.rows)}{result.done ? '' : '…'}</span>
      </button>
    {/each}
  </div>

  <div class="panel" hidden={tab.activeResult !== 'messages'}>
    {#if tab.messages.length === 0}
      <p class="hint muted">
        Run a statement with Ctrl+Enter, or the whole script with Ctrl+Shift+Enter. Results appear here.
      </p>
    {:else}
      <ol class="messages">
        {#each tab.messages as line (line.index)}
          <li class={line.state}>
            <span class="status" aria-hidden="true">{line.state === 'ok' ? '✓' : line.state === 'error' ? '✗' : '…'}</span>
            <div class="body">
              {#if line.sql}<code>{line.sql}</code>{/if}
              <div class="outcome">
                {line.text}
                {#if line.elapsedMs !== null}<span class="muted">&middot; {formatDuration(line.elapsedMs)}</span>{/if}
              </div>
            </div>
          </li>
        {/each}
      </ol>
    {/if}
  </div>

  {#each tab.results as result (result.id)}
    <div class="panel grid-panel" hidden={tab.activeResult !== result.id}>
      <div class="grid-area">
        <Grid
          {result}
          onactivate={(row, column) => showCell(result, row, column)}
          onselect={(row, column) => (selection[result.id] = `Row ${formatCount(row + 1)}, ${result.columns[column].name}`)}
        />
      </div>
      <footer>
        <span>
          {formatCount(result.rows)} {result.rows === 1 ? 'row' : 'rows'}{result.done ? '' : ', still loading…'}
        </span>
        {#if result.truncated}
          <span class="limit">Row limit reached. More rows exist; raise the limit and run again to fetch them.</span>
        {/if}
        {#if result.elapsedMs !== null}<span class="muted">{formatDuration(result.elapsedMs)}</span>{/if}
        <span class="spacer"></span>
        {#if selection[result.id]}<span class="muted">{selection[result.id]}</span>{/if}
        <span class="muted">Double-click a cell to see its full value</span>
      </footer>
    </div>
  {/each}
</section>

<dialog bind:this={viewer} class="viewer" aria-labelledby="cell-title" onclose={() => (openCell = null)}>
  {#if openCell}
    <header>
      <h2 id="cell-title">{openCell.column} <span class="muted">row {formatCount(openCell.row + 1)}</span></h2>
      <span class="muted">
        {openCell.value.text === null ? 'NULL' : `${formatCount(openCell.value.bytes)} bytes`}
        {#if openCell.value.truncated}(showing the first part only){/if}
      </span>
    </header>
    {#if openCell.value.text === null}
      <p class="null muted">NULL</p>
    {:else}
      <pre>{openCell.pretty ?? openCell.value.text}</pre>
    {/if}
    <div class="buttons">
      <button onclick={copyOpenCell} disabled={openCell.value.text === null}>Copy</button>
      <button class="primary" onclick={() => viewer?.close()}>Close</button>
    </div>
  {/if}
</dialog>

<style>
  .results {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .tabs {
    flex: none;
    display: flex;
    gap: 2px;
    padding: 4px 8px 0;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }

  .tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 5px 5px 0 0;
    background: transparent;
    padding: 4px 10px;
  }

  .tabs button.active {
    background: var(--bg);
    border-color: var(--border);
    margin-bottom: -1px;
    padding-bottom: 5px;
    font-weight: 600;
  }

  .badge.error {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--danger);
  }

  .panel {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .grid-panel {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .grid-area {
    flex: 1;
    min-height: 0;
  }

  footer {
    flex: none;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 4px 10px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
  }

  footer .limit {
    color: var(--warn-text);
    background: var(--warn-bg);
    border: 1px solid var(--warn-border);
    border-radius: 4px;
    padding: 0 6px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .spacer {
    flex: 1;
  }

  .hint {
    padding: 14px 16px;
    margin: 0;
  }

  .messages {
    list-style: none;
    margin: 0;
    padding: 6px 0;
  }

  .messages li {
    display: flex;
    gap: 10px;
    padding: 6px 14px;
  }

  .messages li + li {
    border-top: 1px solid var(--border);
  }

  .status {
    flex: none;
    width: 14px;
    font-weight: 700;
  }

  li.ok .status {
    color: var(--ok);
  }

  li.error .status,
  li.error .outcome {
    color: var(--danger);
  }

  .body {
    min-width: 0;
  }

  code {
    display: block;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 4.5em;
    overflow: hidden;
    color: var(--muted);
  }

  .outcome {
    overflow-wrap: anywhere;
    user-select: text;
  }

  .viewer {
    width: min(760px, calc(100vw - 32px));
  }

  .viewer header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px 8px;
  }

  .viewer h2 {
    margin: 0;
    font-size: 15px;
  }

  pre,
  .null {
    margin: 0 16px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 55vh;
    overflow: auto;
    user-select: text;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px 16px;
  }
</style>
