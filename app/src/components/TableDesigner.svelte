<script lang="ts" module>
  // Each designer needs ids of its own for its suggestion lists; several can be open as tabs at once.
  let designers = 0;
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { backend } from '../lib/backend';
  import type { EditorTab, WorkspaceState } from '../lib/state.svelte';
  import { asUiError, type Charset, type ColumnSpec, type IndexSpec, type TableSpec } from '../lib/types';

  interface Props {
    workspace: WorkspaceState;
    /** The tab this designer lives in. Its `design` says which table, or that a new one is being made. */
    tab: EditorTab;
    /** Runs the reviewed SQL; `schema` is the schema it changes. Resolves to an error message, or null on success. */
    onapply: (sql: string, schema: string | null) => Promise<string | null>;
  }

  let { workspace, tab, onapply }: Props = $props();

  const uid = `designer-${++designers}`;

  // The designer's working rows. `pk` is a form convenience that is folded
  // back into the table's primary key when the spec is built.
  interface ColumnRow extends ColumnSpec {
    pk: boolean;
  }
  interface KeyRow {
    name: string;
    /** `table`, or `schema.table` for a table in another schema. */
    reference: string;
    /** One column of this table and the column it points at, per row. */
    pairs: { column: string; referenced: string }[];
    onUpdate: string;
    onDelete: string;
    originalName: string | null;
  }

  let original = $state<TableSpec | null>(null);
  let name = $state('');
  let targetSchema = $state('');
  let engine = $state('');
  let charset = $state('');
  let collation = $state('');
  let comment = $state('');
  let columns = $state<ColumnRow[]>([]);
  let indexes = $state<IndexSpec[]>([]);
  let foreignKeys = $state<KeyRow[]>([]);
  let section = $state<'columns' | 'indexes' | 'keys'>('columns');

  let loading = $state(true);
  let loadError = $state<string | null>(null);
  /** The form as it was loaded, to tell whether anything has been changed. */
  let baseline = $state('');
  let sql = $state('');
  let problem = $state<string | null>(null);
  let applying = $state(false);
  let applyError = $state<string | null>(null);
  let applied = $state(false);

  let schemas = $state<string[]>([]);
  let tableNames = $state<string[]>([]);
  let charsets = $state<Charset[]>([]);
  let collations = $state<string[]>([]);
  /** Columns of tables that foreign keys point at, by `schema.table`: all of them, and the primary key. */
  let referenced = $state<Record<string, { all: string[]; key: string[] }>>({});
  let columnsBody = $state<HTMLTableSectionElement | undefined>();

  const creating = $derived(original === null);
  const columnNames = $derived(columns.map((c) => c.name.trim()).filter(Boolean));
  const collationOptions = $derived(collation && !collations.includes(collation) ? [collation, ...collations] : collations);

  const types = [
    'INT', 'BIGINT', 'SMALLINT', 'TINYINT', 'INT UNSIGNED', 'BIGINT UNSIGNED', 'DECIMAL(10,2)', 'DOUBLE', 'FLOAT',
    'VARCHAR(50)', 'VARCHAR(100)', 'VARCHAR(255)', 'CHAR(10)', 'TEXT', 'MEDIUMTEXT', 'LONGTEXT',
    'DATE', 'DATETIME', 'TIMESTAMP', 'TIME', 'YEAR', 'BOOLEAN', 'JSON', 'BLOB', 'LONGBLOB', 'BINARY(16)',
  ];
  const engines = ['InnoDB', 'MyISAM', 'MEMORY', 'ARCHIVE', 'CSV', 'Aria'];
  const rules = ['RESTRICT', 'CASCADE', 'SET NULL', 'NO ACTION', 'SET DEFAULT'];

  function blankColumn(): ColumnRow {
    return {
      name: '', dataType: '', nullable: true, default: null, onUpdate: null, autoIncrement: false,
      comment: '', collation: null, originalName: null, locked: false, pk: false,
    };
  }

  /** Splits `schema.table` or plain `table`. */
  function reference(text: string): { schema: string; table: string } {
    const clean = text.trim().replaceAll('`', '');
    const dot = clean.indexOf('.');
    return dot < 0 ? { schema: '', table: clean } : { schema: clean.slice(0, dot).trim(), table: clean.slice(dot + 1).trim() };
  }

  /** The character set a collation belongs to: the longest known name it starts with. */
  function charsetOf(value: string): string {
    let found = '';
    for (const option of charsets) {
      if ((value === option.name || value.startsWith(option.name + '_')) && option.name.length > found.length) found = option.name;
    }
    return found;
  }

  async function loadCollations() {
    collations = charset ? await backend.collations(workspace.info.id, charset).catch(() => []) : [];
  }

  async function charsetChanged() {
    await loadCollations();
    collation = charsets.find((c) => c.name === charset)?.defaultCollation ?? '';
  }

  function referencedId(key: KeyRow): string | null {
    const to = reference(key.reference);
    return to.table ? `${to.schema || targetSchema.trim()}.${to.table}` : null;
  }

  /** Reads the columns of the table a foreign key points at, for the suggestions. */
  async function loadReferenced(key: KeyRow): Promise<void> {
    const id = referencedId(key);
    if (!id || referenced[id]) return;
    const to = reference(key.reference);
    try {
      const found = await backend.tableColumns(workspace.info.id, to.schema || targetSchema.trim(), to.table);
      referenced[id] = { all: found.map((c) => c.name), key: found.filter((c) => c.key === 'PRI').map((c) => c.name) };
    } catch {
      // Suggestions only; the column can still be typed.
    }
  }

  /** Once the table referred to is chosen, its primary key is the likely target. */
  async function referenceChanged(key: KeyRow) {
    await loadReferenced(key);
    const id = referencedId(key);
    const likely = id ? (referenced[id]?.key ?? []) : [];
    key.pairs.forEach((pair, n) => {
      if (!pair.referenced && likely[n]) pair.referenced = likely[n];
    });
  }

  async function load() {
    const design = tab.design;
    if (!design || design.kind !== 'table') return;
    loading = true;
    loadError = null;
    try {
      if (design.table === null) {
        original = null;
        name = '';
        targetSchema = design.schema;
        engine = '';
        charset = '';
        collation = '';
        collations = [];
        comment = '';
        columns = [{ ...blankColumn(), name: 'id', dataType: 'INT', nullable: false, autoIncrement: true, pk: true }];
        indexes = [];
        foreignKeys = [];
      } else {
        const spec = await backend.tableSpec(workspace.info.id, design.schema, design.table);
        if (!spec) throw new Error(`The table ${design.table} no longer exists.`);
        original = spec;
        name = spec.name;
        targetSchema = spec.schema;
        engine = spec.engine ?? '';
        collation = spec.collation ?? '';
        charset = charsetOf(collation);
        await loadCollations();
        comment = spec.comment;
        columns = spec.columns.map((c) => ({ ...c, pk: spec.primaryKey.includes(c.name) }));
        indexes = spec.indexes.map((i) => ({ ...i, columns: [...i.columns] }));
        foreignKeys = spec.foreignKeys.map((k) => ({
          name: k.name,
          reference: k.referencedSchema && k.referencedSchema !== spec.schema ? `${k.referencedSchema}.${k.referencedTable}` : k.referencedTable,
          pairs: k.columns.map((column, n) => ({ column, referenced: k.referencedColumns[n] ?? '' })),
          onUpdate: k.onUpdate,
          onDelete: k.onDelete,
          originalName: k.originalName,
        }));
        for (const key of foreignKeys) void loadReferenced(key);
      }
      baseline = JSON.stringify(build());
      backend
        .schemaObjects(workspace.info.id, targetSchema)
        .then((objects) => (tableNames = objects.tables.filter((t) => !t.isView).map((t) => t.name)))
        .catch(() => {
          // Only used for suggestions.
        });
    } catch (e) {
      loadError = asUiError(e).message;
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    // Lists for the pickers. The designer works without them.
    backend
      .schemaList(workspace.info.id)
      .then((list) => (schemas = list))
      .catch(() => {});
    charsets = await backend.charsets(workspace.info.id).catch(() => []);
    await load();
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
      indexes: indexes.map((i) => ({ ...i, columns: [...i.columns] })),
      foreignKeys: foreignKeys.map((k) => {
        const to = reference(k.reference);
        const pairs = k.pairs.filter((p) => p.column || p.referenced.trim());
        return {
          name: k.name.trim(),
          columns: pairs.map((p) => p.column),
          referencedSchema: to.schema,
          referencedTable: to.table,
          referencedColumns: pairs.map((p) => p.referenced.trim()),
          onUpdate: k.onUpdate,
          onDelete: k.onDelete,
          originalName: k.originalName,
        };
      }),
      engine: engine.trim() || null,
      collation: collation.trim() || null,
      comment,
    };
  }

  /** Things the form can say more plainly than the SQL builder would. */
  function formProblem(spec: TableSpec): string | null {
    if (creating && spec.schema === '') return 'Choose the schema to create the table in.';
    const half = spec.foreignKeys.find((k) => k.columns.some((c, n) => (c === '') !== (k.referencedColumns[n] === '')));
    if (half) return `Foreign key ${half.name || '(new)'}: choose a column on both sides of each pair.`;
    return null;
  }

  // Keep the SQL preview, and the tab's "changed" mark, in step with the form.
  $effect(() => {
    if (loading || loadError) return;
    const spec = build();
    tab.designDirty = JSON.stringify(spec) !== baseline;
    const mine = formProblem(spec);
    if (mine) {
      sql = '';
      problem = mine;
      return;
    }
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

  // ---------- columns ----------

  /** Renaming a column also renames it wherever an index or foreign key uses it. */
  function renameColumn(column: ColumnRow, next: string) {
    const previous = column.name.trim();
    column.name = next;
    if (!previous) return;
    for (const index of indexes) {
      if (!index.locked) index.columns = index.columns.map((c) => (c === previous ? next.trim() : c));
    }
    for (const key of foreignKeys) {
      for (const pair of key.pairs) if (pair.column === previous) pair.column = next.trim();
    }
  }

  /** Removing a column also takes it out of indexes, and removes foreign keys that used it. */
  function removeColumn(position: number) {
    const gone = columns[position].name.trim();
    columns.splice(position, 1);
    if (!gone) return;
    for (const index of indexes) {
      if (!index.locked) index.columns = index.columns.filter((c) => c !== gone);
    }
    indexes = indexes.filter((i) => i.locked || i.columns.length > 0 || i.originalName === null);
    foreignKeys = foreignKeys.filter((k) => !k.pairs.some((p) => p.column === gone));
  }

  /**
   * Existing columns keep their place: the designer does not move them.
   * A new table's columns, and columns being added, can go anywhere.
   */
  function movable(column: ColumnRow): boolean {
    return creating || column.originalName === null;
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

  function addColumn(preset: Partial<ColumnRow> = {}) {
    columns.push({ ...blankColumn(), ...preset });
    if (!preset.name) void focusColumn(columns.length - 1);
  }

  /** Enter moves to the next column, adding one after the last. */
  function columnKey(e: KeyboardEvent, position: number) {
    if (e.key !== 'Enter' || !(e.target instanceof HTMLInputElement) || e.target.type !== 'text') return;
    e.preventDefault();
    if (position === columns.length - 1) addColumn();
    else void focusColumn(position + 1);
  }

  // ---------- indexes and foreign keys ----------

  function addIndex(unique = false) {
    indexes.push({ name: '', columns: [], unique, originalName: null, locked: false });
  }

  function addIndexColumn(index: IndexSpec, column: string) {
    if (!column || index.columns.includes(column)) return;
    index.columns.push(column);
    // A sensible name, if none has been typed yet.
    if (index.name.trim() === '') index.name = `${index.unique ? 'uq' : 'idx'}_${index.columns.join('_')}`.slice(0, 64);
  }

  function addForeignKey() {
    foreignKeys.push({ name: '', reference: '', pairs: [{ column: '', referenced: '' }], onUpdate: '', onDelete: '', originalName: null });
  }

  // ---------- applying ----------

  async function apply() {
    applying = true;
    applied = false;
    const spec = build();
    const error = await onapply(sql, spec.schema || null);
    if (error === null) {
      // The tab now stands for the table as it is on the server, under the name it has there.
      tab.design = { kind: 'table', schema: spec.schema, table: spec.name };
      tab.title = `${spec.name} (table)`;
      applyError = null;
      await load();
      applied = loadError === null;
    } else {
      // More than one statement: the earlier ones may have run before one failed.
      applyError = sql.includes(';\n\nALTER TABLE')
        ? `${error} The first statement may already have run. Use Revert to see the table as it now stands.`
        : error;
    }
    applying = false;
  }

  async function revert() {
    applyError = null;
    applied = false;
    await load();
  }
</script>

<div class="designer">
  {#if loading && baseline === ''}
    <p class="muted pad">Loading the table&hellip;</p>
  {:else if loadError}
    <div class="pad">
      <p class="error-text" role="alert">{loadError}</p>
      <button onclick={revert}>Try Again</button>
    </div>
  {:else}
    <!-- Enter never applies: SQL that changes a table runs only when Apply is clicked. -->
    <form onsubmit={(e) => e.preventDefault()}>
      <div class="head">
        <label>
          <span>Table name</span>
          <input type="text" bind:value={name} placeholder="new_table" autocomplete="off" spellcheck="false" />
        </label>
        <label>
          <span>Schema</span>
          {#if creating}
            <select bind:value={targetSchema}>
              {#if targetSchema === '' || !schemas.includes(targetSchema)}<option value={targetSchema}>{targetSchema || 'Choose a schema'}</option>{/if}
              {#each schemas as option (option)}<option value={option}>{option}</option>{/each}
            </select>
          {:else}
            <input type="text" value={targetSchema} disabled />
          {/if}
        </label>
        <label>
          <span>Engine</span>
          <input type="text" bind:value={engine} list={`${uid}-engines`} placeholder="Server default" autocomplete="off" spellcheck="false" />
        </label>
        <label>
          <span>Character set</span>
          <select bind:value={charset} onchange={charsetChanged}>
            <option value="">{collation && !charset ? 'As set by the collation' : 'Schema default'}</option>
            {#each charsets as option (option.name)}<option value={option.name}>{option.name}</option>{/each}
          </select>
        </label>
        <label>
          <span>Collation</span>
          <select bind:value={collation} disabled={collationOptions.length === 0}>
            <option value="">{charset ? 'Default for the character set' : 'Schema default'}</option>
            {#each collationOptions as option (option)}<option value={option}>{option}</option>{/each}
          </select>
        </label>
        <label class="wide">
          <span>Comment</span>
          <input type="text" bind:value={comment} autocomplete="off" />
        </label>
      </div>

      <div class="sections" role="tablist" aria-label="Parts of the table">
        <button type="button" role="tab" aria-selected={section === 'columns'} class:active={section === 'columns'} onclick={() => (section = 'columns')}>
          Columns <span class="muted">{columns.length}</span>
        </button>
        <button type="button" role="tab" aria-selected={section === 'indexes'} class:active={section === 'indexes'} onclick={() => (section = 'indexes')}>
          Indexes <span class="muted">{indexes.length}</span>
        </button>
        <button type="button" role="tab" aria-selected={section === 'keys'} class:active={section === 'keys'} onclick={() => (section = 'keys')}>
          Foreign keys <span class="muted">{foreignKeys.length}</span>
        </button>
      </div>

      <div class="body">
        {#if section === 'columns'}
          <table>
            <thead>
              <tr>
                <th class="wide-cell">Name</th><th class="type-cell">Type</th>
                <th class="box" title="Primary key">PK</th><th class="box" title="Not null: a value is required">NN</th><th class="box" title="Auto increment">AI</th>
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
                  <td><input type="text" value={column.name} oninput={(e) => renameColumn(column, e.currentTarget.value)} disabled={column.locked} placeholder="column_name" aria-label="Column name" autocomplete="off" spellcheck="false" /></td>
                  <td><input type="text" bind:value={column.dataType} list={`${uid}-types`} disabled={column.locked} placeholder="type" aria-label="Type" autocomplete="off" spellcheck="false" /></td>
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
          <div class="under">
            <button type="button" onclick={() => addColumn()}>Add Column</button>
            <button type="button" disabled={columnNames.includes('created_at')} title="Adds created_at, set when a row is inserted" onclick={() => addColumn({ name: 'created_at', dataType: 'TIMESTAMP', nullable: false, default: 'CURRENT_TIMESTAMP' })}>
              + created_at
            </button>
            <button type="button" disabled={columnNames.includes('updated_at')} title="Adds updated_at, set whenever a row changes" onclick={() => addColumn({ name: 'updated_at', dataType: 'TIMESTAMP', nullable: false, default: 'CURRENT_TIMESTAMP', onUpdate: 'CURRENT_TIMESTAMP' })}>
              + updated_at
            </button>
            <span class="muted note">Enter in a column moves to the next one, and adds one after the last.</span>
          </div>
        {:else if section === 'indexes'}
          {#if indexes.length === 0}
            <p class="muted note empty">No indexes besides the primary key.</p>
          {:else}
            <table>
              <thead><tr><th class="name">Name</th><th class="box">Unique</th><th>Columns, in order</th><th></th></tr></thead>
              <tbody>
                {#each indexes as index, i (index)}
                  <tr class:locked={index.locked} title={index.locked ? 'This kind of index is shown, but not changed by the designer' : ''}>
                    <td class="name"><input type="text" bind:value={index.name} disabled={index.locked} placeholder="index_name" aria-label="Index name" autocomplete="off" spellcheck="false" /></td>
                    <td class="box"><input type="checkbox" bind:checked={index.unique} disabled={index.locked} aria-label="Unique" /></td>
                    <td>
                      <div class="chips">
                        {#each index.columns as column, n (column)}
                          <span class="chip">
                            {column}
                            {#if !index.locked}
                              <button type="button" class="quiet" aria-label={`Take ${column} out of the index`} title="Take out of the index" onclick={() => index.columns.splice(n, 1)}>&#215;</button>
                            {/if}
                          </span>
                        {/each}
                        {#if !index.locked}
                          <select
                            value=""
                            aria-label="Add a column to the index"
                            onchange={(e) => {
                              addIndexColumn(index, e.currentTarget.value);
                              e.currentTarget.value = '';
                            }}
                          >
                            <option value="">Add column&hellip;</option>
                            {#each columnNames.filter((c) => !index.columns.includes(c)) as option (option)}<option value={option}>{option}</option>{/each}
                          </select>
                        {/if}
                      </div>
                    </td>
                    <td class="row-actions">
                      <button type="button" class="quiet" disabled={index.locked} aria-label={`Remove index ${index.name}`} title="Remove index" onclick={() => indexes.splice(i, 1)}>&#215;</button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
          <div class="under">
            <button type="button" onclick={() => addIndex()}>Add Index</button>
            <button type="button" onclick={() => addIndex(true)}>Add Unique Index</button>
            <span class="muted note">The order of the columns matters. To change it, take a column out and add it again.</span>
          </div>
        {:else}
          {#if foreignKeys.length === 0}
            <p class="muted note empty">No foreign keys.</p>
          {:else}
            <table>
              <thead>
                <tr><th class="name">Name</th><th class="name">Refers to table</th><th>Column &rarr; column there</th><th>On delete</th><th>On update</th><th></th></tr>
              </thead>
              <tbody>
                {#each foreignKeys as key, k (key)}
                  {@const there = referenced[referencedId(key) ?? '']?.all ?? []}
                  <tr>
                    <td class="name"><input type="text" bind:value={key.name} placeholder="named by the server" aria-label="Foreign key name" autocomplete="off" spellcheck="false" /></td>
                    <td class="name">
                      <input type="text" bind:value={key.reference} list={`${uid}-tables`} placeholder="table or schema.table" aria-label="Table referred to" onchange={() => void referenceChanged(key)} autocomplete="off" spellcheck="false" />
                    </td>
                    <td>
                      {#each key.pairs as pair, p (pair)}
                        <div class="pair">
                          <select bind:value={pair.column} aria-label="Column of this table">
                            <option value="">Column&hellip;</option>
                            {#if pair.column && !columnNames.includes(pair.column)}<option value={pair.column}>{pair.column}</option>{/if}
                            {#each columnNames as option (option)}<option value={option}>{option}</option>{/each}
                          </select>
                          <span aria-hidden="true">&rarr;</span>
                          <input type="text" bind:value={pair.referenced} list={`${uid}-there-${k}`} placeholder="column there" aria-label="Column in the table referred to" autocomplete="off" spellcheck="false" />
                          <button type="button" class="quiet" disabled={key.pairs.length === 1} aria-label="Remove this pair of columns" title="Remove this pair" onclick={() => key.pairs.splice(p, 1)}>&#215;</button>
                        </div>
                      {/each}
                      <datalist id={`${uid}-there-${k}`}>
                        {#each there as option (option)}<option value={option}></option>{/each}
                      </datalist>
                      <button type="button" class="quiet small" title="For a key made of more than one column" onclick={() => key.pairs.push({ column: '', referenced: '' })}>+ another column</button>
                    </td>
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
                    <td class="row-actions">
                      <button type="button" class="quiet" aria-label={`Remove foreign key ${key.name}`} title="Remove foreign key" onclick={() => foreignKeys.splice(k, 1)}>&#215;</button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
          <div class="under">
            <button type="button" disabled={columnNames.length === 0} onclick={addForeignKey}>Add Foreign Key</button>
          </div>
        {/if}
      </div>

      <datalist id={`${uid}-types`}>
        {#each types as type}<option value={type}></option>{/each}
      </datalist>
      <datalist id={`${uid}-engines`}>
        {#each engines as option}<option value={option}></option>{/each}
      </datalist>
      <datalist id={`${uid}-tables`}>
        {#each tableNames as option (option)}<option value={option}></option>{/each}
      </datalist>

      <div class="sql">
        <div class="sql-head">
          <span class="muted">SQL that will run</span>
          {#if !creating}<span class="muted note">Checks and partitioning are not shown here and are left as they are.</span>{/if}
        </div>
        <pre class:waiting={!sql}>{sql || problem || ''}</pre>
        <div class="actions">
          {#if applyError}
            <span class="error-text" role="alert">{applyError}</span>
          {:else if applied && !tab.designDirty}
            <span class="done" role="status">Applied. This is the table as it now stands.</span>
          {/if}
          <span class="spacer"></span>
          <button type="button" disabled={applying || (!tab.designDirty && !applyError)} title="Discard the changes and read the table again" onclick={revert}>Revert</button>
          <button type="button" class="primary" disabled={!sql || applying} onclick={() => void apply()}>{applying ? 'Applying…' : creating ? 'Create Table' : 'Apply Changes'}</button>
        </div>
      </div>
    </form>
  {/if}
</div>

<style>
  .designer {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  form {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .pad {
    padding: 14px 16px;
  }

  .head {
    flex: none;
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 8px 12px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }

  .head .wide {
    grid-column: 1 / -1;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  label > span {
    font-size: 12px;
    color: var(--muted);
  }

  .sections {
    flex: none;
    display: flex;
    gap: 2px;
    padding: 4px 8px 0;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }

  .sections button {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 5px 5px 0 0;
    background: transparent;
    padding: 4px 12px;
  }

  .sections button.active {
    background: var(--bg);
    border-color: var(--border);
    margin-bottom: -1px;
    padding-bottom: 5px;
    font-weight: 600;
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 12px 10px;
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
    background: var(--bg);
    padding: 8px 6px 4px;
    white-space: nowrap;
    border-bottom: 1px solid var(--border);
  }

  td {
    padding: 2px 4px;
    vertical-align: top;
  }

  td input[type='text'],
  td select {
    width: 100%;
    min-width: 70px;
    padding: 3px 6px;
  }

  .box {
    text-align: center;
    width: 40px;
  }

  td.box {
    vertical-align: middle;
  }

  .name {
    width: 22%;
  }

  /* The name and the type are what gets read; they get the room. */
  .wide-cell {
    width: 24%;
  }

  .type-cell {
    width: 22%;
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

  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 1px 2px 1px 8px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--panel);
  }

  .chip button {
    padding: 0 4px;
    font-size: 14px;
    line-height: 16px;
  }

  .chips select {
    width: auto;
    min-width: 130px;
  }

  .pair {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 3px;
  }

  .pair select,
  .pair input[type='text'] {
    flex: 1;
    min-width: 90px;
  }

  td button.small {
    font-size: 12px;
    padding: 1px 6px;
  }

  .under {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding-top: 8px;
  }

  .note {
    font-size: 12px;
  }

  .empty {
    margin: 0;
    padding: 12px 6px 0;
  }

  .sql {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 40%;
    padding: 8px 12px 10px;
    border-top: 1px solid var(--border);
    background: var(--panel);
  }

  .sql-head {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }

  pre {
    flex: 1;
    margin: 0;
    padding: 8px 10px;
    min-height: 54px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
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

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .actions .spacer {
    flex: 1;
  }

  .actions .error-text,
  .actions .done {
    overflow-wrap: anywhere;
  }

  .done {
    color: var(--ok);
  }

  p {
    margin: 0 0 8px;
    overflow-wrap: anywhere;
  }
</style>
