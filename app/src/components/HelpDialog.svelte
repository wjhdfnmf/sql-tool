<script lang="ts">
  import { onMount } from 'svelte';
  import { app } from '../lib/state.svelte';

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  let dialog: HTMLDialogElement;

  onMount(() => dialog.showModal());

  const groups: { title: string; rows: [string, string][] }[] = [
    {
      title: 'Running queries',
      rows: [
        ['Execute the statement under the cursor', 'Ctrl+Enter'],
        ['Execute the selection, or everything', 'Ctrl+Shift+Enter'],
        ['Explain the statement under the cursor', 'Ctrl+Alt+X'],
      ],
    },
    {
      title: 'Editor',
      rows: [
        ['New query tab', 'Ctrl+T'],
        ['Close tab', 'Ctrl+W'],
        ['Open script', 'Ctrl+O'],
        ['Save script, Save as', 'Ctrl+S, Ctrl+Shift+S'],
        ['Find and replace', 'Ctrl+F'],
        ['Comment or uncomment lines', 'Ctrl+/'],
        ['Reformat query', 'Ctrl+B'],
      ],
    },
    {
      title: 'Results',
      rows: [
        ['Select cells', 'Drag, Shift+Click, Shift+Arrows'],
        ['Select rows, columns, everything', 'Shift+Space, Ctrl+Space, Ctrl+A'],
        ['Copy, copy with column names', 'Ctrl+C, Ctrl+Shift+C'],
        ['Fit a column to its contents', "Double-click the header's edge"],
        ['More ways to copy, filter and sort', 'Right-click'],
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

<dialog bind:this={dialog} onclose={onclose} aria-labelledby="help-title">
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
    </div>
    <div class="buttons">
      <button class="primary" onclick={() => dialog.close()}>Close</button>
    </div>
  </div>
</dialog>

<style>
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
