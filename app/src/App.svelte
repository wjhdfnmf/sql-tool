<script lang="ts">
  import { onMount } from 'svelte';
  import Home from './components/Home.svelte';
  import SettingsDialog from './components/SettingsDialog.svelte';
  import Workspace from './components/Workspace.svelte';
  import { backend } from './lib/backend';
  import { runSelfTest } from './lib/selftest';
  import { app } from './lib/state.svelte';
  import { asUiError } from './lib/types';

  let loadError = $state<string | null>(null);

  onMount(async () => {
    try {
      await app.load();
      const selfTest = await backend.selftestConfig();
      if (selfTest) void runSelfTest(selfTest);
    } catch (e) {
      loadError = asUiError(e).message;
    }
  });

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
    <button class="tab settings" title="Themes, timeouts and other settings" onclick={() => (app.showSettings = true)}>Settings</button>
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

<style>
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
  .tab.settings {
    margin-left: auto;
    color: var(--muted);
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
