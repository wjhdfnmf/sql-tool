<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import type { EditorTab, WorkspaceState } from '../lib/state.svelte';
  import { asUiError, type Charset, type SchemaObjects } from '../lib/types';

  interface Props {
    workspace: WorkspaceState;
    /** The tab this editor lives in. Its `design` says which schema, or that a new one is being made. */
    tab: EditorTab;
    /** Runs the reviewed SQL. Resolves to an error message, or null on success. */
    onapply: (sql: string, schema: string | null) => Promise<string | null>;
  }

  let { workspace, tab, onapply }: Props = $props();

  let name = $state('');
  let charset = $state('');
  let collation = $state('');
  /** What was loaded, to tell whether anything has been changed. */
  let loaded = $state({ name: '', charset: '', collation: '' });
  let altering = $state(false);
  let charsets = $state<Charset[]>([]);
  let collations = $state<string[]>([]);
  let contents = $state<SchemaObjects | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let sql = $state('');
  let problem = $state<string | null>(null);
  let applying = $state(false);
  let applyError = $state<string | null>(null);
  let applied = $state(false);

  async function loadCollations() {
    collations = charset ? await backend.collations(workspace.info.id, charset).catch(() => []) : [];
  }

  async function charsetChanged() {
    await loadCollations();
    collation = charsets.find((c) => c.name === charset)?.defaultCollation ?? '';
  }

  async function load() {
    const design = tab.design;
    if (!design || design.kind !== 'schema') return;
    loading = true;
    loadError = null;
    try {
      altering = design.schema !== null;
      name = design.schema ?? '';
      charset = '';
      collation = '';
      contents = null;
      if (design.schema !== null) {
        const defaults = await backend.schemaDefaults(workspace.info.id, design.schema);
        if (!defaults) throw new Error(`The schema ${design.schema} no longer exists.`);
        charset = defaults.charset;
        await loadCollations();
        collation = defaults.collation;
        backend
          .schemaObjects(workspace.info.id, design.schema)
          .then((objects) => (contents = objects))
          .catch(() => {
            // The summary is a convenience.
          });
      } else {
        collations = [];
      }
      loaded = { name, charset, collation };
    } catch (e) {
      loadError = asUiError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    charsets = await backend.charsets(workspace.info.id).catch(() => []);
    await load();
  });

  const changed = $derived(name !== loaded.name || charset !== loaded.charset || collation !== loaded.collation);

  // Keep the SQL preview, and the tab's "changed" mark, in step with the form.
  $effect(() => {
    if (loading || loadError) return;
    tab.designDirty = changed;
    if (altering && !changed) {
      sql = '';
      problem = 'Nothing has been changed.';
      return;
    }
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
    applied = false;
    const created = name.trim();
    const error = await onapply(sql, created || null);
    if (error === null) {
      // The tab now stands for the schema as it is on the server.
      tab.design = { kind: 'schema', schema: created };
      tab.title = `${created} (schema)`;
      applyError = null;
      await load();
      applied = loadError === null;
    } else {
      applyError = error;
    }
    applying = false;
  }

  async function revert() {
    applyError = null;
    applied = false;
    await load();
  }

  const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
</script>

<div class="editor">
  {#if loading && loaded.name === '' && !altering}
    <p class="muted">Loading&hellip;</p>
  {:else if loadError}
    <p class="error-text" role="alert">{loadError}</p>
    <div><button onclick={revert}>Try Again</button></div>
  {:else}
    <!-- Enter never applies: SQL that changes a schema runs only when the button is clicked. -->
    <form onsubmit={(e) => e.preventDefault()}>
      <h2>{altering ? `Schema ${loaded.name}` : 'New schema'}</h2>

      <label>
        <span>Name</span>
        <input type="text" bind:value={name} disabled={altering} placeholder="new_schema" autocomplete="off" spellcheck="false" />
        {#if altering}<span class="muted note">MySQL and MariaDB cannot rename a schema.</span>{/if}
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
      <p class="muted note">
        The character set and collation are the defaults for tables created in this schema from now on. Changing them does not
        convert tables that already exist.
      </p>

      {#if contents}
        <p class="muted">
          Holds {count(contents.tables.filter((t) => !t.isView).length, 'table', 'tables')},
          {count(contents.tables.filter((t) => t.isView).length, 'view', 'views')},
          {count(contents.routines.length, 'routine', 'routines')},
          {count(contents.triggers.length, 'trigger', 'triggers')} and
          {count(contents.events.length, 'event', 'events')}.
        </p>
      {/if}

      <div>
        <span class="muted">SQL that will run</span>
        <pre class:waiting={!sql}>{sql || problem || ''}</pre>
      </div>

      <div class="actions">
        {#if applyError}
          <span class="error-text" role="alert">{applyError}</span>
        {:else if applied && !changed}
          <span class="done" role="status">Applied.</span>
        {/if}
        <span class="spacer"></span>
        <button type="button" disabled={applying || (!changed && !applyError)} title="Discard the changes" onclick={revert}>Revert</button>
        <button type="button" class="primary" disabled={!sql || applying} onclick={() => void apply()}>
          {applying ? 'Applying…' : altering ? 'Apply Changes' : 'Create Schema'}
        </button>
      </div>
    </form>
  {/if}
</div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    overflow: auto;
    padding: 16px 18px;
    background: var(--bg);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 640px;
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

  label > span:first-child {
    font-size: 12px;
    color: var(--muted);
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  .note {
    font-size: 12px;
  }

  pre {
    margin: 4px 0 0;
    padding: 8px 10px;
    min-height: 40px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  /* Not SQL yet: a note on what is missing. */
  pre.waiting {
    color: var(--muted);
    font-family: inherit;
  }

  p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }

  .done {
    color: var(--ok);
  }
</style>
