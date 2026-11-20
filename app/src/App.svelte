<script lang="ts">
  import { onMount } from 'svelte';
  import HelpDialog from './components/HelpDialog.svelte';
  import Home from './components/Home.svelte';
  import SettingsDialog from './components/SettingsDialog.svelte';
  import WorkbenchImport from './components/WorkbenchImport.svelte';
  import Workspace from './components/Workspace.svelte';
  import { backend } from './lib/backend';
  import { faults } from './lib/faults.svelte';
  import { runSelfTest } from './lib/selftest';
  import { settings } from './lib/settings.svelte';
  import { runSmoke } from './lib/smoke';
  import { app } from './lib/state.svelte';
  import { asUiError } from './lib/types';

  let loadError = $state<string | null>(null);

  onMount(async () => {
    try {
      await app.load();
      await backend.onCloseRequested(() => (askingToClose = true));
      if (app.info?.smoke) {
        void runSmoke();
        return;
      }
      const selfTest = await backend.selftestConfig();
      if (selfTest) void runSelfTest(selfTest);
    } catch (e) {
      loadError = asUiError(e).message;
    }
  });

  // ---------- closing the window ----------

  /** What closing the window now would lose, a line for each kind of thing. Empty when nothing would be. */
  const atRisk = $derived.by(() => {
    let running = 0;
    let edits = 0;
    let designs = 0;
    let scripts = 0;
    for (const workspace of app.workspaces) {
      // A script that will be there again the next time is not lost.
      const remembered = settings.current.rememberTabs && !!workspace.info.profileId;
      for (const tab of workspace.tabs) {
        if (tab.running) running++;
        if (tab.designDirty) designs++;
        if (tab.filePath && tab.dirty && !remembered) scripts++;
        for (const result of tab.results) if ((result.pendingEdits ?? 0) > 0) edits++;
      }
    }
    const lines: string[] = [];
    if (running > 0) lines.push(running === 1 ? 'A statement is still running. It will be stopped where it is.' : `${running} statements are still running. They will be stopped where they are.`);
    if (edits > 0) lines.push(edits === 1 ? 'A result has changes that were not applied.' : `${edits} results have changes that were not applied.`);
    if (designs > 0) lines.push(designs === 1 ? 'A table or schema being designed has changes that were not applied.' : `${designs} tables or schemas being designed have changes that were not applied.`);
    if (scripts > 0) lines.push(scripts === 1 ? 'A script has changes that were not saved to its file.' : `${scripts} scripts have changes that were not saved to their files.`);
    return lines;
  });
  let askingToClose = $state(false);

  // The shell asks before closing only while it has been told there is
  // something to lose. Told when that changes, not on every key typed.
  const guarded = $derived(atRisk.length > 0);
  $effect(() => {
    void backend.closeGuard(guarded).catch(() => {});
  });

  function answerClose(close: boolean) {
    askingToClose = false;
    void backend.closeAnswer(close).catch(() => {});
  }

  function modal(node: HTMLDialogElement) {
    node.showModal();
  }

  function tagColor(color: string | null): string {
    return color ? `var(--tag-${color})` : 'var(--muted)';
  }
</script>

<div class="shell">
  <nav class="connections" aria-label="Open connections">
    <button class="tab" class:active={app.active === 'home'} onclick={() => (app.active = 'home')}>
      Connections
    </button>
    {#each app.workspaces as workspace (workspace.info.id)}
      <div class="tab-group" class:active={app.active === workspace.info.id}>
        <button class="tab" onclick={() => (app.active = workspace.info.id)}>
          <span class="tag-dot" style:background={tagColor(workspace.info.color)}></span>
          {workspace.info.name}
        </button>
        <button
          class="close quiet"
          aria-label={`Disconnect from ${workspace.info.name}`}
          title="Disconnect"
          onclick={() => app.close(workspace)}>&#215;</button
        >
      </div>
    {/each}
    <!-- The connection in front shows its own buttons here: its menu, and Navigator and Output. -->
    <span class="end" bind:this={app.toolsHost} hidden={app.active === 'home'}></span>
    {#if app.active === 'home'}
      <span class="end">
        <button class="tab" title="Themes, timeouts and other settings" onclick={() => (app.showSettings = true)}>Settings</button>
        <button class="tab" title="Keyboard shortcuts and where things are" onclick={() => (app.showHelp = true)}>Help</button>
      </span>
    {/if}
  </nav>

  {#if loadError}
    <p class="load-error error-text" role="alert">Could not start: {loadError}</p>
  {/if}

  <div class="view" hidden={app.active !== 'home'}>
    <Home />
  </div>
  {#each app.workspaces as workspace (workspace.info.id)}
    <div class="view" hidden={app.active !== workspace.info.id}>
      <Workspace {workspace} />
    </div>
  {/each}
</div>

{#if app.showSettings}
  <SettingsDialog onclose={() => (app.showSettings = false)} />
{/if}

{#if app.showImport}
  <WorkbenchImport onclose={() => (app.showImport = false)} />
{/if}

{#if app.showHelp}
  <HelpDialog onclose={() => (app.showHelp = false)} />
{/if}

{#if askingToClose}
  <dialog
    use:modal
    aria-labelledby="close-title"
    oncancel={(e) => {
      e.preventDefault();
      answerClose(false);
    }}
  >
    <div class="closing">
      <h2 id="close-title">Close SQL Tool?</h2>
      {#if atRisk.length > 0}
        <ul>
          {#each atRisk as line (line)}<li>{line}</li>{/each}
        </ul>
      {:else}
        <p class="muted">There is nothing left that closing would lose.</p>
      {/if}
      <div class="buttons">
        <button class="primary" onclick={() => answerClose(false)}>Keep Open</button>
        <button class="danger" onclick={() => answerClose(true)}>Close Anyway</button>
      </div>
    </div>
  </dialog>
{/if}

{#if faults.message}
  <div class="fault" role="alert">
    <div>
      <strong>Something went wrong.</strong>
      <span>{faults.message}</span>
      <p class="muted">
        What you were doing may not have happened. If the app stops responding, close and reopen it.
        {#if app.info}A note of this is in <code>crash.log</code> in {app.info.dataDir}.{/if}
      </p>
    </div>
    <button onclick={() => (faults.message = null)}>Dismiss</button>
  </div>
{/if}

<style>
  .closing {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 440px;
    max-width: 100%;
    padding: 20px;
  }

  .closing h2 {
    margin: 0;
    font-size: 16px;
  }

  .closing ul {
    margin: 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .closing p {
    margin: 0;
  }

  .closing .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }

  /* Along the bottom, over whatever is there: it is about the app, not about one screen. */
  .fault {
    position: fixed;
    left: 50%;
    bottom: 34px;
    transform: translateX(-50%);
    z-index: 50;
    display: flex;
    align-items: flex-start;
    gap: 14px;
    width: min(620px, calc(100vw - 32px));
    padding: 10px 12px;
    border: 1px solid var(--warn-border);
    border-radius: 8px;
    background: var(--warn-bg);
    color: var(--warn-text);
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.25);
  }

  .fault div {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .fault p {
    margin: 4px 0 0;
    font-size: 12px;
    color: inherit;
    opacity: 0.85;
  }

  .fault button {
    flex: none;
  }

  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .connections {
    flex: none;
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 8px 0;
    background: var(--panel-2);
    border-bottom: 1px solid var(--border);
    /* No scroll bar: tabs that do not fit go onto another row. */
    flex-wrap: wrap;
  }

  /* Pushed to the far end of the strip. */
  .end {
    display: flex;
    gap: 2px;
    margin-left: auto;
  }

  .end .tab {
    color: var(--muted);
  }

  .end {
    align-items: center;
  }

  .tab,
  .tab-group {
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 6px 6px 0 0;
    background: transparent;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 12px;
  }

  .tab-group {
    display: flex;
    align-items: center;
  }

  .tab-group .tab {
    border: none;
    padding-right: 4px;
  }

  .tab.active,
  .tab-group.active {
    background: var(--bg);
    border-color: var(--border);
    margin-bottom: -1px;
    padding-bottom: 1px;
    font-weight: 600;
  }

  .close {
    padding: 0 7px;
    margin-right: 4px;
    font-size: 15px;
    line-height: 20px;
    color: var(--muted);
  }

  .view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .load-error {
    margin: 12px 16px;
  }
</style>
