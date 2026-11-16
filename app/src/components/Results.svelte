<script lang="ts">
  import type { EditorTab, WorkspaceState } from '../lib/state.svelte';
  import { formatCount, formatDuration } from '../lib/types';
  import ResultView from './ResultView.svelte';

  interface Props {
    workspace: WorkspaceState;
    tab: EditorTab;
  }

  let { workspace, tab }: Props = $props();
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
        Run the statement under the cursor with Ctrl+Enter, or the selection or whole script with Ctrl+Shift+Enter.
        Results appear here.
      </p>
    {:else}
      <ol class="messages">
        {#each tab.messages as line (line.id)}
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
    <div class="panel result-panel" hidden={tab.activeResult !== result.id}>
      <ResultView {workspace} {tab} {result} />
    </div>
  {/each}
</section>

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

  .result-panel {
    overflow: hidden;
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
</style>
