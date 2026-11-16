<script lang="ts">
  import { backend } from '../lib/backend';
  import { asUiError, formatDuration, type HistoryEntry } from '../lib/types';

  interface Props {
    /** Changes whenever a run finishes, so the list reloads. */
    refreshKey: unknown;
    /** Put the statement into the active editor. */
    oninsert: (sql: string) => void;
  }

  let { refreshKey, oninsert }: Props = $props();

  let entries = $state<HistoryEntry[]>([]);
  let search = $state('');
  let error = $state<string | null>(null);
  let confirmClear = $state(false);

  async function load(term: string) {
    try {
      entries = await backend.historyList(term.trim() || null, 200);
      error = null;
    } catch (e) {
      error = asUiError(e).message;
    }
  }

  $effect(() => {
    // Re-run when the search text or the refresh key changes; a short delay
    // avoids a request per keystroke.
    void refreshKey;
    const term = search;
    const timer = setTimeout(() => load(term), 150);
    return () => clearTimeout(timer);
  });

  async function clear() {
    if (!confirmClear) {
      confirmClear = true;
      setTimeout(() => (confirmClear = false), 4000);
      return;
    }
    confirmClear = false;
    await backend.historyClear();
    await load(search);
  }

  function when(ms: number): string {
    const date = new Date(ms);
    const today = new Date().toDateString() === date.toDateString();
    return today
      ? date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
      : date.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' + date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
</script>

<div class="history">
  <div class="toolbar">
    <input type="search" bind:value={search} placeholder="Search history" aria-label="Search history" />
    <button class="quiet danger" onclick={clear} disabled={entries.length === 0 && !confirmClear}>
      {confirmClear ? 'Confirm' : 'Clear'}
    </button>
  </div>
  {#if error}<p class="error-text note" role="alert">{error}</p>{/if}
  {#if entries.length === 0 && !error}
    <p class="muted note">{search.trim() ? 'No statements match.' : 'Statements you run appear here.'}</p>
  {/if}
  <ul>
    {#each entries as entry, index (index)}
      <li>
        <button class="entry" title="Insert into the editor" onclick={() => oninsert(entry.sql)}>
          <code>{entry.sql}</code>
          <span class="meta muted">
            <span class={entry.ok ? 'ok' : 'failed'} aria-hidden="true">{entry.ok ? '✓' : '✗'}</span>
            {when(entry.atMs)} &middot; {formatDuration(entry.durationMs)} &middot; {entry.profileName}
          </span>
          {#if !entry.ok}<span class="meta error-text">{entry.summary}</span>{/if}
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .toolbar {
    flex: none;
    display: flex;
    gap: 4px;
    padding: 8px;
  }

  .toolbar input {
    flex: 1;
  }

  ul {
    flex: 1;
    min-height: 0;
    overflow: auto;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .entry {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 7px 10px;
    border: none;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    background: transparent;
    text-align: left;
    white-space: normal;
  }

  .entry:hover {
    background: var(--panel-2);
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .meta {
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .ok {
    color: var(--ok);
  }

  .failed {
    color: var(--danger);
  }

  .note {
    margin: 0;
    padding: 6px 10px;
  }
</style>
