<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, type ObjectAction, type ObjectKind } from '../lib/types';

  interface Props {
    action: ObjectAction;
    kind: ObjectKind;
    schema: string;
    /** The object's name; for a schema, the schema name again. */
    name: string;
    onclose: () => void;
    /** Runs the SQL. Resolves to an error message, or null on success. */
    onapply: (sql: string) => Promise<string | null>;
  }

  let { action, kind, schema, name, onclose, onapply }: Props = $props();

  let dialog: HTMLDialogElement;
  let sql = $state('');
  let problem = $state<string | null>(null);
  let newName = $state('');
  let typed = $state('');
  let applying = $state(false);
  let applyError = $state<string | null>(null);

  const what = $derived(action === 'dropSchema' ? 'schema' : kind === 'procedure' ? 'stored procedure' : kind);
  const title = $derived(
    action === 'rename' ? `Rename table ${name}` : action === 'truncate' ? `Truncate table ${name}` : `Drop ${what} ${name}`,
  );
  const consequence = $derived(
    action === 'dropSchema'
      ? 'This deletes the schema and every table and all data in it. It cannot be undone.'
      : action === 'truncate'
        ? 'This deletes every row in the table. It cannot be undone.'
        : action === 'drop'
          ? kind === 'table'
            ? 'This deletes the table and all its data. It cannot be undone.'
            : `This deletes the ${what}. It cannot be undone.`
          : null,
  );
  // Dropping a whole schema asks for its name to be typed, as a deliberate step.
  const confirmed = $derived(action !== 'dropSchema' || typed === name);

  onMount(() => {
    dialog.showModal();
    newName = name;
  });

  $effect(() => {
    const request = [action, kind, schema, name, action === 'rename' ? newName : null] as const;
    let stale = false;
    backend
      .objectDdl(...request)
      .then((text) => {
        if (!stale) {
          sql = text;
          problem = null;
        }
      })
      .catch((e) => {
        if (!stale) {
          sql = '';
          problem = asUiError(e).message;
        }
      });
    return () => {
      stale = true;
    };
  });

  async function apply() {
    applying = true;
    applyError = await onapply(sql);
    applying = false;
  }
</script>

<dialog bind:this={dialog} oncancel={onclose} aria-labelledby="confirm-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      if (sql && confirmed && (action !== 'rename' || newName.trim() !== name)) void apply();
    }}
  >
    <h2 id="confirm-title">{title}</h2>
    {#if consequence}<p class="warning-box">{consequence}</p>{/if}

    {#if action === 'rename'}
      <label>
        <span>New name</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" bind:value={newName} autocomplete="off" spellcheck="false" autofocus />
      </label>
    {/if}
    {#if action === 'dropSchema'}
      <label>
        <span>Type <strong>{name}</strong> to confirm</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" bind:value={typed} autocomplete="off" spellcheck="false" autofocus />
      </label>
    {/if}

    <div>
      <span class="muted">SQL that will run</span>
      <pre>{sql || problem || ''}</pre>
    </div>
    {#if applyError}<p class="error-text" role="alert">{applyError}</p>{/if}

    <div class="buttons">
      <button type="button" onclick={onclose}>Cancel</button>
      <button
        type="submit"
        class={action === 'rename' ? 'primary' : 'danger'}
        disabled={!sql || !confirmed || applying || (action === 'rename' && newName.trim() === name)}
      >
        {applying ? 'Working…' : action === 'rename' ? 'Rename' : action === 'truncate' ? 'Truncate' : 'Drop'}
      </button>
    </div>
  </form>
</dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    width: 480px;
    max-width: 100%;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  pre {
    margin: 4px 0 0;
    padding: 8px 10px;
    min-height: 34px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
