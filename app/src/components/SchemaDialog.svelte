<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, type Charset } from '../lib/types';

  interface Props {
    workspaceId: number;
    /** The schema to alter, or null to create a new one. */
    schema: string | null;
    onclose: () => void;
    /** Runs the reviewed SQL. Resolves to an error message, or null on success. */
    onapply: (sql: string) => Promise<string | null>;
  }

  let { workspaceId, schema, onclose, onapply }: Props = $props();

  let dialog: HTMLDialogElement;
  let name = $state('');
  let charset = $state('');
  let collation = $state('');
  let charsets = $state<Charset[]>([]);
  let collations = $state<string[]>([]);
  let sql = $state('');
  let problem = $state<string | null>(null);
  let applying = $state(false);
  let applyError = $state<string | null>(null);

  const altering = $derived(schema !== null);

  onMount(async () => {
    dialog.showModal();
    name = schema ?? '';
    try {
      charsets = await backend.charsets(workspaceId);
      if (schema) {
        const defaults = await backend.schemaDefaults(workspaceId, schema);
        if (defaults) {
          charset = defaults.charset;
          await loadCollations();
          collation = defaults.collation;
        }
      }
    } catch (e) {
      applyError = asUiError(e).message;
    }
  });

  async function loadCollations() {
    collations = charset ? await backend.collations(workspaceId, charset).catch(() => []) : [];
  }

  async function charsetChanged() {
    await loadCollations();
    collation = charsets.find((c) => c.name === charset)?.defaultCollation ?? '';
  }

  // Keep the SQL preview in step with the form.
  $effect(() => {
    const request = [altering, name, charset || null, collation || null] as const;
    let stale = false;
    backend
      .schemaDdl(...request)
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

<dialog bind:this={dialog} oncancel={onclose} aria-labelledby="schema-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      if (sql) void apply();
    }}
  >
    <h2 id="schema-title">{altering ? `Alter schema ${schema}` : 'Create schema'}</h2>

    <label>
      <span>Name</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input type="text" bind:value={name} disabled={altering} autocomplete="off" spellcheck="false" autofocus={!altering} />
    </label>
    <div class="pair">
      <label>
        <span>Character set</span>
        <select bind:value={charset} onchange={charsetChanged}>
          <option value="">Server default</option>
          {#each charsets as option (option.name)}
            <option value={option.name}>{option.name}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>Collation</span>
        <select bind:value={collation} disabled={!charset}>
          <option value="">Default for the character set</option>
          {#each collations as option (option)}
            <option value={option}>{option}</option>
          {/each}
        </select>
      </label>
    </div>

    <div>
      <span class="muted">SQL that will run</span>
      <pre>{sql || problem || ''}</pre>
    </div>

    {#if applyError}<p class="error-text" role="alert">{applyError}</p>{/if}

    <div class="buttons">
      <button type="button" onclick={onclose}>Cancel</button>
      <button type="submit" class="primary" disabled={!sql || applying}>{applying ? 'Applying…' : 'Apply'}</button>
    </div>
  </form>
</dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    width: 520px;
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
    min-width: 0;
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
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
