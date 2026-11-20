<script lang="ts">
  import { onMount } from 'svelte';
  import { loadLicences } from '../lib/licences';
  import { keys } from '../lib/platform';
  import { app } from '../lib/state.svelte';

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let dialog: HTMLDialogElement;

  onMount(() => dialog.showModal());

  // The licences are long, so they are loaded when asked for.
  let licences = $state<string | null>(null);
  let loadingLicences = $state(false);
  let licenceError = $state<string | null>(null);

  async function showLicences() {
    loadingLicences = true;
    licenceError = null;
    try {
      licences = await loadLicences();
    } catch {
      licenceError = 'The licences could not be loaded. They are also listed in THIRD-PARTY.md with the source.';
    } finally {
      loadingLicences = false;
    }
  }

  const groups: { title: string; rows: [string, string][] }[] = [
    {
      title: 'Running queries',
      rows: [
        ['Execute the statement under the cursor', keys('Mod+Enter')],
        ['Execute the selection, or everything', keys('Mod+Shift+Enter')],
        ['Explain the statement under the cursor', keys('Mod+Alt+X')],
        ['Run a long query as many short ones', 'In Slices, with {start} and {end} in the query'],
      ],
    },
    {
      title: 'Editor',
      rows: [
        ['New query tab', keys('Mod+T')],
        ['Close tab', keys('Mod+W')],
        ['Open script', keys('Mod+O')],
        ['Save script, Save as', keys('Mod+S, Mod+Shift+S')],
        ['Find and replace', keys('Mod+F')],
        ['Comment or uncomment lines', keys('Mod+/')],
        ['Reformat query', keys('Mod+B')],
      ],
    },
    {
      title: 'Results',
      rows: [
        ['Select cells', 'Drag, Shift+Click, Shift+Arrows'],
        ['Select rows, columns, everything', keys('Shift+Space, Ctrl+Space, Mod+A')],
        ['Copy, copy with column names', keys('Mod+C, Mod+Shift+C')],
        ['Fit a column to its contents', "Double-click the header's edge"],
        ['More ways to copy, filter and sort', 'Right-click'],
      ],
    },
    {
      title: 'Graphs',
      rows: [
        ['Draw a graph of the result in front', 'Graph, beside the menu button'],
        ['Put a column on an axis, or colour by it', 'Drag it there, or click it'],
        ['Draw a line at a value: a limit, a target', 'Right-click an axis, or the chart'],
        ['Add a trend line', 'Right-click the chart'],
        ['Name an axis, add a title', 'Right-click the chart, or Options'],
        ['Capability figures (Cp, Cpk)', 'Mark a line as a specification limit'],
        ['Start a graph over', 'Reset'],
        ['Zoom in to a range', 'Drag across the chart'],
        ['Show the whole range again', 'Double-click the chart, or All'],
        ['Back to the query', 'Esc'],
      ],
    },
    {
      title: 'Schemas',
      rows: [
        ['See a table\'s columns and indexes', 'Click it'],
        ['Show a table\'s rows', 'Double-click it'],
        ['Make a schema the default', 'Double-click it'],
        ['Everything else', 'Right-click, or Shift+F10'],
        ['Find a table', 'Type table, or schema.table, in the filter'],
      ],
    },
  ];
</script>

<dialog bind:this={dialog} onclose={onclose} aria-labelledby="help-title" class:wide={licences !== null}>
  <div class="help">
    <h2 id="help-title">SQL Tool {app.info?.version ?? ''}</h2>
    <p class="muted">A free, open-source SQL client for MySQL and MariaDB. Early alpha.</p>
    <div class="scroll">
      {#each groups as group (group.title)}
        <h3>{group.title}</h3>
        <table>
          <tbody>
            {#each group.rows as [what, how] (what)}
              <tr><td>{what}</td><td>{how}</td></tr>
            {/each}
          </tbody>
        </table>
      {/each}
      <h3>Where things are</h3>
      <p>
        In a connection, the <strong>&#9776;</strong> button at the end of the row of connection tabs holds what has no button of its
        own: statement history, details of the connection, creating schemas and tables, server information, settings and this help.
        <strong>Navigator</strong> and <strong>Output</strong> beside it show and hide the side panes. Settings and Help are also on
        the Connections page.
      </p>
      <h3>About</h3>
      <p>
        SQL Tool is free software under the Apache License 2.0. Its source, new versions and the place to report a problem:
        <span class="where">github.com/wjhdfnmf/sql-tool</span>
      </p>
      {#if app.info?.dataDir}
        <p>
          Settings, connections and history are kept in <span class="where">{app.info.dataDir}</span>. If the app closes by itself or
          reports that something went wrong, a note of it is in <code>crash.log</code> there.
        </p>
      {/if}
      <p>
        It is built with other open-source software, each piece under its own licence.
        {#if licences === null}
          <button onclick={showLicences} disabled={loadingLicences}>{loadingLicences ? 'Loading…' : 'Show the licences'}</button>
        {/if}
      </p>
      {#if licenceError}<p class="error-text" role="alert">{licenceError}</p>{/if}
      {#if licences !== null}
        <pre class="licences">{licences}</pre>
      {/if}
    </div>
    <div class="buttons">
      <button class="primary" onclick={() => dialog.close()}>Close</button>
    </div>
  </div>
</dialog>

<style>
  /* The licences are written 80 characters wide. */
  dialog.wide {
    max-width: min(760px, calc(100vw - 32px));
  }

  dialog.wide .help {
    width: 720px;
  }

  .where {
    font-family: var(--mono);
    font-size: 12px;
    overflow-wrap: anywhere;
    user-select: all;
  }

  .licences {
    margin: 4px 0 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 11.5px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .help {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 20px;
    width: 540px;
    max-width: 100%;
    max-height: calc(100vh - 80px);
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  h3 {
    margin: 12px 0 2px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  p {
    margin: 0;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-right: 6px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  td {
    padding: 3px 0;
    border-bottom: 1px solid var(--border);
  }

  td:last-child {
    text-align: right;
    font-family: var(--mono);
    font-size: 12px;
    padding-left: 16px;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    margin-top: 4px;
  }
</style>
