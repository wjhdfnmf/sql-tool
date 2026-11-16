<script lang="ts">
  import { backend } from '../lib/backend';
  import type { GridOverlay } from '../lib/grid';
  import { separator, type MenuItem } from '../lib/menu';
  import { decodeCells } from '../lib/rows';
  import type { EditorTab, ResultTab, WorkspaceState } from '../lib/state.svelte';
  import {
    asUiError,
    formatCount,
    formatDuration,
    type CellChange,
    type CellValue,
    type EditTarget,
    type ExportFormat,
    type RowEdit,
  } from '../lib/types';
  import Grid from './Grid.svelte';
  import Menu from './Menu.svelte';

  interface Props {
    workspace: WorkspaceState;
    tab: EditorTab;
    result: ResultTab;
  }

  let { workspace, tab, result }: Props = $props();

  // ---------- sorting and filtering ----------

  let filter = $state('');
  let sort = $state<{ column: number; descending: boolean } | null>(null);
  /** Rows visible after sorting and filtering; null until a view is applied. */
  let visibleRows = $state<number | null>(null);
  let viewVersion = $state(0);
  let note = $state<string | null>(null);
  let position = $state('');
  let contextMenu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null);

  const rows = $derived(visibleRows ?? result.rows);

  async function applyView() {
    try {
      visibleRows = await backend.resultView(result.id, filter.trim() || null, sort?.column ?? null, sort?.descending ?? false);
      viewVersion++;
      note = null;
    } catch (e) {
      note = asUiError(e).message;
    }
  }

  let filterTimer: ReturnType<typeof setTimeout> | undefined;
  function filterChanged() {
    clearTimeout(filterTimer);
    filterTimer = setTimeout(applyView, 250);
  }

  function sortBy(column: number) {
    if (!result.done) return;
    if (pending > 0) {
      note = 'Apply or discard the pending changes before sorting.';
      return;
    }
    // Ascending, then descending, then back to the order the rows arrived in.
    if (sort?.column !== column) sort = { column, descending: false };
    else if (!sort.descending) sort = { column, descending: true };
    else sort = null;
    void applyView();
  }

  // ---------- export ----------

  let format = $state<ExportFormat>('csv');
  let exportTable = $state('');
  let exporting = $state(false);

  async function exportRows() {
    exporting = true;
    try {
      const done = await backend.exportResult(result.id, format, exportTable.trim() || target?.table || 'table_name');
      if (done) note = `Exported ${formatCount(done.rows)} rows to ${done.path}`;
    } catch (e) {
      note = asUiError(e).message;
    } finally {
      exporting = false;
    }
  }

  // ---------- editing ----------

  let target = $state<EditTarget | null>(null);
  let whyNot = $state<string | null>(null);
  /** Changed cells, keyed `row:column`; null means set to NULL. */
  let updates = $state<Record<string, string | null>>({});
  let deletes = $state<number[]>([]);
  /** New rows: column index to value. */
  let inserts = $state<Record<number, string>[]>([]);

  const pending = $derived(Object.keys(updates).length + deletes.length + inserts.length);
  const overlay = $derived<GridOverlay>({
    cells: new Map(Object.entries(updates)),
    deletedRows: new Set(deletes),
  });

  let askedTarget = false;
  $effect(() => {
    if (!result.done || askedTarget) return;
    askedTarget = true;
    backend
      .editTarget(workspace.info.id, result.id)
      .then((t) => (target = t))
      .catch((e) => (whyNot = asUiError(e).message));
  });

  function buildEdits(): RowEdit[] {
    const byRow = new Map<number, CellChange[]>();
    for (const [key, value] of Object.entries(updates)) {
      const [row, column] = key.split(':').map(Number);
      if (deletes.includes(row)) continue;
      if (!byRow.has(row)) byRow.set(row, []);
      byRow.get(row)!.push({ column, value });
    }
    return [
      ...[...byRow].map(([row, changes]): RowEdit => ({ type: 'update', row, changes })),
      ...deletes.map((row): RowEdit => ({ type: 'delete', row })),
      ...inserts.map(
        (values): RowEdit => ({
          type: 'insert',
          values: Object.entries(values).map(([column, value]) => ({ column: Number(column), value })),
        }),
      ),
    ];
  }

  function discard() {
    updates = {};
    deletes = [];
    inserts = [];
    note = null;
  }

  function toggleDelete(row: number) {
    deletes = deletes.includes(row) ? deletes.filter((r) => r !== row) : [...deletes, row];
  }

  // Cell viewer and editor.
  interface OpenCell {
    row: number;
    column: number;
    value: CellValue;
    text: string;
    isNull: boolean;
    editable: boolean;
    pretty: string | null;
  }
  let openCell = $state<OpenCell | null>(null);

  async function showCell(row: number, column: number) {
    try {
      const value = await backend.getCell(result.id, row, column);
      const key = `${row}:${column}`;
      const staged = key in updates;
      const current = staged ? updates[key] : value.text;
      let pretty: string | null = null;
      if (value.text !== null && result.columns[column].kind === 'json' && !value.truncated) {
        try {
          pretty = JSON.stringify(JSON.parse(value.text), null, 2);
        } catch {
          // Not valid JSON after all; show it as it is.
        }
      }
      openCell = {
        row,
        column,
        value,
        text: current ?? '',
        isNull: current === null,
        // A value too large to load in full cannot be edited safely.
        editable: !!target && target.columns[column] !== null && !value.truncated && !deletes.includes(row),
        pretty,
      };
    } catch (e) {
      note = asUiError(e).message;
    }
  }

  function stageCell() {
    if (!openCell) return;
    const key = `${openCell.row}:${openCell.column}`;
    const next = openCell.isNull ? null : openCell.text;
    if (next === openCell.value.text) delete updates[key];
    else updates[key] = next;
    openCell = null;
  }

  // New row.
  let newRow = $state<Record<number, string> | null>(null);

  function stageInsert() {
    if (!newRow) return;
    const values = Object.fromEntries(Object.entries(newRow).filter(([, value]) => value !== ''));
    if (Object.keys(values).length === 0) return;
    inserts = [...inserts, values];
    newRow = null;
  }

  // Review and apply.
  let review = $state<{ sql: string[]; error: string | null; applying: boolean } | null>(null);

  async function openReview() {
    try {
      review = { sql: await backend.previewEdits(result.id, buildEdits()), error: null, applying: false };
    } catch (e) {
      note = asUiError(e).message;
    }
  }

  async function applyEdits() {
    if (!review) return;
    review.applying = true;
    review.error = null;
    try {
      await backend.applyEdits(workspace.info.id, tab.id, result.id, buildEdits());
      review = null;
      discard();
      // Run the statement again so the grid shows what is now in the table.
      await workspace.run(tab, result.sql, { kind: 'all' });
    } catch (e) {
      if (review) {
        review.error = asUiError(e).message;
        review.applying = false;
      }
    }
  }

  // ---------- right-click ----------

  async function copyRow(row: number) {
    try {
      const cells = decodeCells(await backend.getRows(result.id, row, 1), result.columns.length).values;
      await navigator.clipboard.writeText(cells.map((c) => c ?? 'NULL').join('\t'));
    } catch {
      // Clipboard access can be refused.
    }
  }

  async function copyCell(row: number, column: number) {
    try {
      await navigator.clipboard.writeText((await backend.getCell(result.id, row, column)).text ?? 'NULL');
    } catch {
      // Clipboard access can be refused.
    }
  }

  function cellMenu(row: number, column: number): MenuItem[] {
    const items: MenuItem[] = [
      { label: 'Copy Cell', shortcut: 'Ctrl+C', action: () => void copyCell(row, column) },
      { label: 'Copy Row', action: () => void copyRow(row) },
      { label: 'View Value…', action: () => void showCell(row, column) },
    ];
    if (target) {
      items.push(
        separator,
        { label: 'Edit Cell…', disabled: target.columns[column] === null || deletes.includes(row), action: () => void showCell(row, column) },
        { label: deletes.includes(row) ? 'Keep Row' : 'Delete Row', action: () => toggleDelete(row) },
        { label: 'Add Row…', action: () => (newRow = {}) },
      );
    }
    return items;
  }

  function modal(node: HTMLDialogElement) {
    node.showModal();
  }
</script>

<div class="result">
  <div class="bar">
    <input
      type="search"
      class="filter"
      bind:value={filter}
      oninput={filterChanged}
      disabled={!result.done || pending > 0}
      placeholder="Filter rows"
      aria-label="Filter rows"
      title={pending > 0 ? 'Apply or discard the pending changes first' : 'Show only rows that contain this text'}
    />
    <span class="divider"></span>
    <select bind:value={format} aria-label="Export format">
      <option value="csv">CSV</option>
      <option value="tsv">TSV</option>
      <option value="json">JSON</option>
      <option value="sql">SQL INSERT</option>
    </select>
    {#if format === 'sql'}
      <input type="text" class="table" bind:value={exportTable} placeholder={target?.table ?? 'table name'} aria-label="Table name for the INSERT statements" />
    {/if}
    <button disabled={!result.done || exporting} onclick={exportRows} title="Save the rows shown to a file">
      {exporting ? 'Exporting…' : 'Export…'}
    </button>
    <span class="divider"></span>
    {#if target}
      <span class="muted" title={`Changes go to ${target.schema}.${target.table}`}>Editable</span>
      <button onclick={() => (newRow = {})}>Add Row</button>
      {#if pending > 0}
        <span class="pending">{pending} pending {pending === 1 ? 'change' : 'changes'}</span>
        <button class="primary" onclick={openReview}>Review and Apply&hellip;</button>
        <button onclick={discard}>Discard</button>
      {/if}
    {:else if whyNot}
      <span class="muted" title={whyNot}>Read-only result</span>
    {/if}
  </div>

  <div class="grid-area">
    <Grid
      {result}
      {rows}
      {viewVersion}
      {sort}
      {overlay}
      onactivate={showCell}
      onselect={(row, column) => (position = `Row ${formatCount(row + 1)}, ${result.columns[column].name}`)}
      onsort={sortBy}
      oncontext={(row, column, x, y) => (contextMenu = { items: cellMenu(row, column), x, y })}
    />
  </div>

  <footer>
    <span>
      {#if visibleRows !== null && visibleRows !== result.rows}
        {formatCount(visibleRows)} of {formatCount(result.rows)} rows shown
      {:else}
        {formatCount(result.rows)} {result.rows === 1 ? 'row' : 'rows'}{result.done ? '' : ', still loading…'}
      {/if}
    </span>
    {#if result.truncated}
      <span class="limit">Row limit reached. More rows exist; raise the limit and run again to fetch them.</span>
    {/if}
    {#if result.elapsedMs !== null}<span class="muted">{formatDuration(result.elapsedMs)}</span>{/if}
    {#if note}<span class="note" role="status">{note}</span>{/if}
    <span class="spacer"></span>
    {#if position}<span class="muted">{position}</span>{/if}
    <span class="muted">Click a header to sort. Double-click a cell to {target ? 'edit' : 'view'} it.</span>
  </footer>
</div>

{#if contextMenu}
  <Menu items={contextMenu.items} label="Cell actions" x={contextMenu.x} y={contextMenu.y} onclose={() => (contextMenu = null)} />
{/if}

{#if openCell}
  {@const cell = openCell}
  <dialog use:modal class="wide" aria-labelledby="cell-title" onclose={() => (openCell = null)}>
    <header>
      <h2 id="cell-title">{result.columns[cell.column].name} <span class="muted">row {formatCount(cell.row + 1)}</span></h2>
      <span class="muted">
        {cell.value.text === null ? 'NULL' : `${formatCount(cell.value.bytes)} bytes`}
        {#if cell.value.truncated}(showing the first part only){/if}
      </span>
    </header>
    {#if cell.editable}
      <textarea bind:value={openCell.text} disabled={openCell.isNull} rows="8" aria-label="Value" spellcheck="false"></textarea>
      <label class="check"><input type="checkbox" bind:checked={openCell.isNull} /> Set to NULL</label>
    {:else if cell.value.text === null}
      <p class="value muted">NULL</p>
    {:else}
      <pre class="value">{cell.pretty ?? cell.value.text}</pre>
    {/if}
    <div class="buttons">
      <button onclick={() => void navigator.clipboard.writeText(cell.pretty ?? cell.value.text ?? 'NULL')}>Copy</button>
      <span class="spacer"></span>
      <button onclick={() => (openCell = null)}>{cell.editable ? 'Cancel' : 'Close'}</button>
      {#if cell.editable}<button class="primary" onclick={stageCell}>Stage Change</button>{/if}
    </div>
  </dialog>
{/if}

{#if newRow && target}
  {@const editable = target.columns.map((name, index) => ({ name, index })).filter((c) => c.name !== null)}
  <dialog use:modal aria-labelledby="row-title" onclose={() => (newRow = null)}>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        stageInsert();
      }}
    >
      <header><h2 id="row-title">Add a row to {target.table}</h2></header>
      <p class="muted hint">Leave a field empty to use the column's default.</p>
      <div class="fields">
        {#each editable as column (column.index)}
          <label>
            <span>{column.name}</span>
            <input type="text" bind:value={newRow[column.index]} autocomplete="off" spellcheck="false" />
          </label>
        {/each}
      </div>
      <div class="buttons">
        <span class="spacer"></span>
        <button type="button" onclick={() => (newRow = null)}>Cancel</button>
        <button type="submit" class="primary">Stage Row</button>
      </div>
    </form>
  </dialog>
{/if}

{#if review}
  <dialog use:modal class="wide" aria-labelledby="review-title" onclose={() => (review = null)}>
    <header><h2 id="review-title">Apply {review.sql.length} {review.sql.length === 1 ? 'change' : 'changes'}</h2></header>
    <p class="muted hint">These run together in one transaction. If any fails, none are applied.</p>
    <pre class="value">{review.sql.join('\n')}</pre>
    {#if review.error}<p class="error-text hint" role="alert">{review.error}</p>{/if}
    <div class="buttons">
      <span class="spacer"></span>
      <button onclick={() => (review = null)}>Cancel</button>
      <button class="primary" disabled={review.applying} onclick={applyEdits}>{review.applying ? 'Applying…' : 'Apply'}</button>
    </div>
  </dialog>
{/if}

<style>
  .result {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    white-space: nowrap;
  }

  .bar button,
  .bar select,
  .bar input {
    padding: 2px 8px;
  }

  .filter {
    width: 190px;
  }

  .table {
    width: 130px;
  }

  .divider {
    flex: none;
    width: 1px;
    align-self: stretch;
    margin: 2px 4px;
    background: var(--border);
  }

  .pending {
    color: var(--warn-text);
    background: var(--warn-bg);
    border: 1px solid var(--warn-border);
    border-radius: 4px;
    padding: 0 6px;
  }

  .grid-area {
    flex: 1;
    min-height: 0;
  }

  footer {
    flex: none;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 4px 10px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
  }

  footer .limit {
    color: var(--warn-text);
    background: var(--warn-bg);
    border: 1px solid var(--warn-border);
    border-radius: 4px;
    padding: 0 6px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .note {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .spacer {
    flex: 1;
  }

  dialog {
    width: min(520px, calc(100vw - 32px));
  }

  dialog.wide {
    width: min(760px, calc(100vw - 32px));
  }

  dialog header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px 8px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }

  .value,
  textarea {
    display: block;
    width: calc(100% - 32px);
    margin: 0 16px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    color: var(--text);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 55vh;
    overflow: auto;
    user-select: text;
  }

  textarea {
    background: var(--bg);
    resize: vertical;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px 16px 0;
  }

  .hint {
    margin: 0 16px 8px;
  }

  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 12px;
    padding: 0 16px;
    max-height: 50vh;
    overflow: auto;
  }

  .fields label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .buttons {
    display: flex;
    gap: 8px;
    padding: 12px 16px 16px;
  }
</style>
