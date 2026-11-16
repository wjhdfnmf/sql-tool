<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, type ColumnSpec, type IndexSpec, type TableSpec } from '../lib/types';

  interface Props {
    workspaceId: number;
    schema: string;
    /** The table to alter, or null to create a new one. */
    table: string | null;
    onclose: () => void;
    /** Runs the reviewed SQL. Resolves to an error message, or null on success. */
    onapply: (sql: string) => Promise<string | null>;
  }

  let { workspaceId, schema, table, onclose, onapply }: Props = $props();

  // The designer's working rows. `pk` and `columnsText` are form
  // conveniences that are folded back into the spec when it is built.
  interface ColumnRow extends ColumnSpec {
    pk: boolean;
  }
  interface IndexRow extends IndexSpec {
    columnsText: string;
  }

  let dialog: HTMLDialogElement;
  let original = $state<TableSpec | null>(null);
  let name = $state('');
  let targetSchema = $state('');
  let engine = $state('');
  let collation = $state('');
  let comment = $state('');
  let columns = $state<ColumnRow[]>([]);
  let indexes = $state<IndexRow[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let sql = $state('');
  let problem = $state<string | null>(null);
  let applying = $state(false);
  let applyError = $state<string | null>(null);

  const types = [
    'INT', 'BIGINT', 'SMALLINT', 'TINYINT', 'DECIMAL(10,2)', 'DOUBLE', 'VARCHAR(100)', 'VARCHAR(255)', 'CHAR(10)',
    'TEXT', 'LONGTEXT', 'DATE', 'DATETIME', 'TIMESTAMP', 'TIME', 'BOOLEAN', 'JSON', 'BLOB',
  ];

  function blankColumn(): ColumnRow {
    return {
      name: '', dataType: '', nullable: true, default: null, onUpdate: null, autoIncrement: false,
      comment: '', collation: null, originalName: null, locked: false, pk: false,
    };
  }

  onMount(async () => {
    dialog.showModal();
    targetSchema = schema;
    try {
      if (table === null) {
        columns = [{ ...blankColumn(), name: 'id', dataType: 'INT', nullable: false, autoIncrement: true, pk: true }];
      } else {
        const spec = await backend.tableSpec(workspaceId, schema, table);
        if (!spec) throw new Error(`The table ${table} no longer exists.`);
        original = spec;
        name = spec.name;
        engine = spec.engine ?? '';
        collation = spec.collation ?? '';
        comment = spec.comment;
        columns = spec.columns.map((c) => ({ ...c, pk: spec.primaryKey.includes(c.name) }));
        indexes = spec.indexes.map((i) => ({ ...i, columnsText: i.columns.join(', ') }));
      }
    } catch (e) {
      loadError = asUiError(e).message;
    } finally {
      loading = false;
    }
  });

  /** The form as a spec, the shape the backend compares and writes out. */
  function build(): TableSpec {
    const keyNames = columns.filter((c) => c.pk).map((c) => c.name.trim());
    // A composite key keeps its original column order unless its members changed.
    let primaryKey = keyNames;
    if (original) {
      const renamed = (old: string) => columns.find((c) => c.originalName === old)?.name.trim();
      const kept = original.primaryKey.map(renamed);
      if (kept.length === keyNames.length && kept.every((n) => n !== undefined && keyNames.includes(n))) {
        primaryKey = kept as string[];
      }
    }
    return {
      schema: targetSchema.trim(),
      name: name.trim(),
      columns: columns.map(({ pk: _pk, ...c }) => ({ ...c, default: c.default?.trim() ? c.default : null })),
      primaryKey,
      indexes: indexes.map(({ columnsText, ...i }) => ({
        ...i,
        columns: i.locked ? i.columns : columnsText.split(',').map((s) => s.trim()).filter(Boolean),
      })),
      engine: engine.trim() || null,
      collation: collation.trim() || null,
      comment,
    };
  }

  // Keep the SQL preview in step with the form.
  $effect(() => {
    if (loading || loadError) return;
    const spec = build();
    let stale = false;
    backend
      .tableDdl($state.snapshot(original), spec)
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

  /** Renaming a column also renames it wherever an index lists it. */
  function renameColumn(column: ColumnRow, next: string) {
    const previous = column.name.trim();
    column.name = next;
    if (!previous) return;
    for (const index of indexes) {
      if (index.locked) continue;
      const parts = index.columnsText.split(',').map((s) => s.trim());
      if (parts.includes(previous)) index.columnsText = parts.map((p) => (p === previous ? next.trim() : p)).join(', ');
    }
  }

  async function apply() {
    applying = true;
    applyError = await onapply(sql);
    applying = false;
  }
</script>

<dialog bind:this={dialog} oncancel={onclose} aria-labelledby="designer-title">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      if (sql) void apply();
    }}
  >
    <h2 id="designer-title">{table === null ? 'Create table' : `Alter table ${table}`}</h2>

    {#if loading}
      <p class="muted">Loading the table&hellip;</p>
    {:else if loadError}
      <p class="error-text" role="alert">{loadError}</p>
    {:else}
      <div class="top">
        <label><span>Table name</span><input type="text" bind:value={name} autocomplete="off" spellcheck="false" /></label>
        <label><span>Schema</span><input type="text" bind:value={targetSchema} disabled={table !== null} autocomplete="off" spellcheck="false" /></label>
        <label><span>Engine</span><input type="text" bind:value={engine} placeholder="Server default" autocomplete="off" spellcheck="false" /></label>
        <label><span>Collation</span><input type="text" bind:value={collation} placeholder="Schema default" autocomplete="off" spellcheck="false" /></label>
        <label class="wide"><span>Comment</span><input type="text" bind:value={comment} autocomplete="off" /></label>
      </div>

      <h3>Columns</h3>
      <div class="scroll">
        <table>
          <thead>
            <tr>
              <th>Name</th><th>Type</th>
              <th title="Primary key">PK</th><th title="Not null">NN</th><th title="Auto increment">AI</th>
              <th>Default</th><th>Comment</th><th></th>
            </tr>
          </thead>
          <tbody>
            {#each columns as column, index (column)}
              <tr class:locked={column.locked} title={column.locked ? 'Generated column: shown, but not changed by the designer' : ''}>
                <td><input type="text" value={column.name} oninput={(e) => renameColumn(column, e.currentTarget.value)} disabled={column.locked} aria-label="Column name" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={column.dataType} list="designer-types" disabled={column.locked} aria-label="Type" autocomplete="off" spellcheck="false" /></td>
                <td class="box"><input type="checkbox" bind:checked={column.pk} disabled={column.locked} aria-label="Primary key" onchange={() => { if (column.pk) column.nullable = false; }} /></td>
                <td class="box"><input type="checkbox" checked={!column.nullable} onchange={(e) => (column.nullable = !e.currentTarget.checked)} disabled={column.locked || column.pk} aria-label="Not null" /></td>
                <td class="box"><input type="checkbox" bind:checked={column.autoIncrement} disabled={column.locked} aria-label="Auto increment" /></td>
                <td><input type="text" value={column.default ?? ''} oninput={(e) => (column.default = e.currentTarget.value)} disabled={column.locked} placeholder="none" aria-label="Default, as SQL" title="As SQL: 0, 'text', CURRENT_TIMESTAMP" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={column.comment} disabled={column.locked} aria-label="Comment" autocomplete="off" /></td>
                <td><button type="button" class="quiet" disabled={column.locked} aria-label={`Remove column ${column.name}`} title="Remove column" onclick={() => columns.splice(index, 1)}>&#215;</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <datalist id="designer-types">
        {#each types as type}<option value={type}></option>{/each}
      </datalist>
      <div><button type="button" onclick={() => columns.push(blankColumn())}>Add Column</button></div>

      <h3>Indexes</h3>
      {#if indexes.length > 0}
        <table>
          <thead><tr><th>Name</th><th>Columns, in order</th><th>Unique</th><th></th></tr></thead>
          <tbody>
            {#each indexes as index, i (index)}
              <tr class:locked={index.locked} title={index.locked ? 'This kind of index is shown, but not changed by the designer' : ''}>
                <td><input type="text" bind:value={index.name} disabled={index.locked} aria-label="Index name" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={index.columnsText} disabled={index.locked} placeholder="column, column" aria-label="Index columns" autocomplete="off" spellcheck="false" /></td>
                <td class="box"><input type="checkbox" bind:checked={index.unique} disabled={index.locked} aria-label="Unique" /></td>
                <td><button type="button" class="quiet" disabled={index.locked} aria-label={`Remove index ${index.name}`} title="Remove index" onclick={() => indexes.splice(i, 1)}>&#215;</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
      <div>
        <button type="button" onclick={() => indexes.push({ name: '', columns: [], unique: false, originalName: null, locked: false, columnsText: '' })}>
          Add Index
        </button>
      </div>

      <div>
        <span class="muted">SQL that will run</span>
        <pre>{sql || problem || ''}</pre>
      </div>
      {#if original}
        <p class="muted note">Foreign keys, checks and partitioning are not shown here and are left as they are.</p>
      {/if}
    {/if}

    {#if applyError}<p class="error-text" role="alert">{applyError}</p>{/if}

    <div class="buttons">
      <button type="button" onclick={onclose}>Cancel</button>
      <button type="submit" class="primary" disabled={!sql || applying}>{applying ? 'Applying…' : 'Apply'}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    max-width: min(980px, calc(100vw - 32px));
    width: 980px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 20px;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    overflow-wrap: anywhere;
  }

  h3 {
    margin: 6px 0 0;
    font-size: 13px;
  }

  .top {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px 12px;
  }

  .top .wide {
    grid-column: 1 / -1;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .scroll {
    max-height: 34vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    position: sticky;
    top: 0;
    z-index: 1;
    text-align: left;
    font-weight: 600;
    color: var(--muted);
    background: var(--panel);
    padding: 4px 6px;
    white-space: nowrap;
  }

  td {
    padding: 2px 4px;
  }

  td input[type='text'] {
    width: 100%;
    min-width: 70px;
    padding: 3px 6px;
  }

  td.box {
    text-align: center;
    width: 34px;
  }

  tr.locked {
    opacity: 0.6;
  }

  td button {
    padding: 0 7px;
    font-size: 15px;
    color: var(--muted);
  }

  pre {
    margin: 4px 0 0;
    padding: 8px 10px;
    min-height: 34px;
    max-height: 22vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  p {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .note {
    font-size: 12px;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
