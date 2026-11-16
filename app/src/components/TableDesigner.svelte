<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, type ColumnSpec, type ForeignKeySpec, type IndexSpec, type TableSpec } from '../lib/types';

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
  interface KeyRow extends ForeignKeySpec {
    columnsText: string;
    /** `table`, or `schema.table` for a table in another schema. */
    referenceText: string;
    referencedText: string;
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
  let foreignKeys = $state<KeyRow[]>([]);
  /** Tables of the schema, offered when choosing what a foreign key refers to. */
  let tableNames = $state<string[]>([]);
  let columnsBody = $state<HTMLTableSectionElement | undefined>();
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

  const rules = ['RESTRICT', 'CASCADE', 'SET NULL', 'NO ACTION', 'SET DEFAULT'];

  const list = (text: string) => text.split(',').map((s) => s.trim().replaceAll('`', '')).filter(Boolean);

  /** Splits `schema.table` or plain `table`. */
  function reference(text: string): { schema: string; table: string } {
    const clean = text.trim().replaceAll('`', '');
    const dot = clean.indexOf('.');
    return dot < 0 ? { schema: '', table: clean } : { schema: clean.slice(0, dot).trim(), table: clean.slice(dot + 1).trim() };
  }

  onMount(async () => {
    dialog.showModal();
    targetSchema = schema;
    backend
      .schemaObjects(workspaceId, schema)
      .then((objects) => (tableNames = objects.tables.filter((t) => !t.isView).map((t) => t.name)))
      .catch(() => {
        // Only used for suggestions.
      });
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
        foreignKeys = spec.foreignKeys.map((k) => ({
          ...k,
          columnsText: k.columns.join(', '),
          referenceText: k.referencedSchema && k.referencedSchema !== spec.schema ? `${k.referencedSchema}.${k.referencedTable}` : k.referencedTable,
          referencedText: k.referencedColumns.join(', '),
        }));
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
        columns: i.locked ? i.columns : list(columnsText),
      })),
      foreignKeys: foreignKeys.map(({ columnsText, referenceText, referencedText, ...k }) => {
        const to = reference(referenceText);
        return {
          ...k,
          name: k.name.trim(),
          columns: list(columnsText),
          referencedSchema: to.schema,
          referencedTable: to.table,
          referencedColumns: list(referencedText),
        };
      }),
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

  /** Renaming a column also renames it wherever an index or foreign key lists it. */
  function renameColumn(column: ColumnRow, next: string) {
    const previous = column.name.trim();
    column.name = next;
    if (!previous) return;
    for (const row of [...indexes.filter((i) => !i.locked), ...foreignKeys]) {
      const parts = list(row.columnsText);
      if (parts.includes(previous)) row.columnsText = parts.map((p) => (p === previous ? next.trim() : p)).join(', ');
    }
  }

  /** Removing a column also takes it out of indexes, and removes foreign keys that used it. */
  function removeColumn(position: number) {
    const gone = columns[position].name.trim();
    columns.splice(position, 1);
    if (!gone) return;
    for (const index of indexes) {
      if (!index.locked) index.columnsText = list(index.columnsText).filter((p) => p !== gone).join(', ');
    }
    indexes = indexes.filter((i) => i.locked || i.columnsText !== '' || i.originalName === null);
    foreignKeys = foreignKeys.filter((k) => !list(k.columnsText).includes(gone));
  }

  /**
   * Existing columns keep their place: the designer does not move them.
   * A new table's columns, and columns being added, can go anywhere.
   */
  function movable(column: ColumnRow): boolean {
    return original === null || column.originalName === null;
  }

  function moveColumn(position: number, by: -1 | 1) {
    const to = position + by;
    if (to < 0 || to >= columns.length) return;
    [columns[position], columns[to]] = [columns[to], columns[position]];
  }

  async function focusColumn(position: number) {
    await tick();
    columnsBody?.querySelectorAll('tr')[position]?.querySelector('input')?.focus();
  }

  function addColumn() {
    columns.push(blankColumn());
    void focusColumn(columns.length - 1);
  }

  /** Enter moves to the next column, adding one after the last. */
  function columnKey(e: KeyboardEvent, position: number) {
    if (e.key !== 'Enter' || !(e.target instanceof HTMLInputElement) || e.target.type !== 'text') return;
    e.preventDefault();
    if (position === columns.length - 1) addColumn();
    else void focusColumn(position + 1);
  }

  function addForeignKey() {
    foreignKeys.push({
      name: '', columns: [], referencedSchema: '', referencedTable: '', referencedColumns: [],
      onUpdate: '', onDelete: '', originalName: null, columnsText: '', referenceText: '', referencedText: '',
    });
  }

  /** Once the table referred to is chosen, its primary key is the likely target. */
  async function suggestReferenced(key: KeyRow) {
    if (key.referencedText.trim() !== '') return;
    const to = reference(key.referenceText);
    if (!to.table) return;
    try {
      const spec = await backend.tableSpec(workspaceId, to.schema || targetSchema.trim(), to.table);
      if (spec && key.referencedText.trim() === '') key.referencedText = spec.primaryKey.join(', ');
    } catch {
      // A suggestion only; the user can type the columns.
    }
  }

  async function apply() {
    applying = true;
    const error = await onapply(sql);
    // More than one statement: the earlier ones may have run before one failed.
    applyError =
      error !== null && sql.includes(';\n\nALTER TABLE')
        ? `${error} The first statement may already have run. Close this and open the table again to see how it stands.`
        : error;
    applying = false;
  }
</script>

<dialog bind:this={dialog} oncancel={onclose} aria-labelledby="designer-title">
  <!-- Enter never applies: SQL that changes a table runs only when Apply is clicked. -->
  <form onsubmit={(e) => e.preventDefault()}>
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
          <tbody bind:this={columnsBody}>
            {#each columns as column, index (column)}
              <tr
                class:locked={column.locked}
                title={column.locked ? 'Generated column: shown, but not changed by the designer' : ''}
                onkeydown={(e) => columnKey(e, index)}
              >
                <td><input type="text" value={column.name} oninput={(e) => renameColumn(column, e.currentTarget.value)} disabled={column.locked} aria-label="Column name" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={column.dataType} list="designer-types" disabled={column.locked} aria-label="Type" autocomplete="off" spellcheck="false" /></td>
                <td class="box"><input type="checkbox" bind:checked={column.pk} disabled={column.locked} aria-label="Primary key" onchange={() => { if (column.pk) column.nullable = false; }} /></td>
                <td class="box"><input type="checkbox" checked={!column.nullable} onchange={(e) => (column.nullable = !e.currentTarget.checked)} disabled={column.locked || column.pk} aria-label="Not null" /></td>
                <td class="box"><input type="checkbox" bind:checked={column.autoIncrement} disabled={column.locked} aria-label="Auto increment" /></td>
                <td><input type="text" value={column.default ?? ''} oninput={(e) => (column.default = e.currentTarget.value)} disabled={column.locked} placeholder="none" aria-label="Default, as SQL" title="As SQL: 0, 'text', CURRENT_TIMESTAMP" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={column.comment} disabled={column.locked} aria-label="Comment" autocomplete="off" /></td>
                <td class="row-actions">
                  <button type="button" class="quiet" disabled={!movable(column) || index === 0} aria-label={`Move column ${column.name} up`} title={movable(column) ? 'Move up' : 'Existing columns keep their place'} onclick={() => moveColumn(index, -1)}>&#9652;</button>
                  <button type="button" class="quiet" disabled={!movable(column) || index === columns.length - 1} aria-label={`Move column ${column.name} down`} title={movable(column) ? 'Move down' : 'Existing columns keep their place'} onclick={() => moveColumn(index, 1)}>&#9662;</button>
                  <button type="button" class="quiet" disabled={column.locked} aria-label={`Remove column ${column.name}`} title="Remove column" onclick={() => removeColumn(index)}>&#215;</button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <datalist id="designer-types">
        {#each types as type}<option value={type}></option>{/each}
      </datalist>
      <div class="under">
        <button type="button" onclick={addColumn}>Add Column</button>
        <span class="muted note">Enter in a column moves to the next one, and adds one after the last.</span>
      </div>

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

      <h3>Foreign keys</h3>
      {#if foreignKeys.length > 0}
        <table>
          <thead>
            <tr>
              <th>Name</th><th>Columns</th><th>Refers to table</th><th>Columns there</th><th>On delete</th><th>On update</th><th></th>
            </tr>
          </thead>
          <tbody>
            {#each foreignKeys as key, k (key)}
              <tr>
                <td><input type="text" bind:value={key.name} placeholder="named by the server" aria-label="Foreign key name" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={key.columnsText} placeholder="column, column" aria-label="Foreign key columns" autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={key.referenceText} list="designer-tables" placeholder="table or schema.table" aria-label="Table referred to" onchange={() => void suggestReferenced(key)} autocomplete="off" spellcheck="false" /></td>
                <td><input type="text" bind:value={key.referencedText} placeholder="column, column" aria-label="Columns in the table referred to" autocomplete="off" spellcheck="false" /></td>
                <td>
                  <select bind:value={key.onDelete} aria-label="On delete">
                    <option value="">Server default</option>
                    {#each rules as rule}<option value={rule}>{rule}</option>{/each}
                  </select>
                </td>
                <td>
                  <select bind:value={key.onUpdate} aria-label="On update">
                    <option value="">Server default</option>
                    {#each rules as rule}<option value={rule}>{rule}</option>{/each}
                  </select>
                </td>
                <td><button type="button" class="quiet" aria-label={`Remove foreign key ${key.name}`} title="Remove foreign key" onclick={() => foreignKeys.splice(k, 1)}>&#215;</button></td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
      <datalist id="designer-tables">
        {#each tableNames as tableName}<option value={tableName}></option>{/each}
      </datalist>
      <div><button type="button" onclick={addForeignKey}>Add Foreign Key</button></div>

      <div>
        <span class="muted">SQL that will run</span>
        <pre>{sql || problem || ''}</pre>
      </div>
      {#if original}
        <p class="muted note">Checks and partitioning are not shown here and are left as they are.</p>
      {/if}
    {/if}

    {#if applyError}<p class="error-text" role="alert">{applyError}</p>{/if}

    <div class="buttons">
      <button type="button" onclick={onclose}>Cancel</button>
      <button type="button" class="primary" disabled={!sql || applying} onclick={() => void apply()}>{applying ? 'Applying…' : 'Apply'}</button>
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

  td.row-actions {
    white-space: nowrap;
    width: 1%;
  }

  td.row-actions button {
    padding: 0 4px;
  }

  td select {
    width: 100%;
    padding: 3px 4px;
  }

  .under {
    display: flex;
    align-items: center;
    gap: 12px;
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
