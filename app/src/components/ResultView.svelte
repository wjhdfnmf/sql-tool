<script lang="ts">
  import { backend } from '../lib/backend';
  import type { CellPosition, GridOverlay } from '../lib/grid';
  import { separator, type MenuItem } from '../lib/menu';
  import { placeIn } from '../lib/place';
  import type { EditorTab, ResultTab, WorkspaceState } from '../lib/state.svelte';
  import {
    asUiError,
    formatCount,
    formatDuration,
    type CellChange,
    type CellRange,
    type CellValue,
    type CopyFormat,
    type EditTarget,
    type ExportFormat,
    type RangeStats,
    type RowEdit,
  } from '../lib/types';
  import Grid from './Grid.svelte';
  import Menu from './Menu.svelte';

  interface Props {
    workspace: WorkspaceState;
    tab: EditorTab;
    result: ResultTab;
    /** The end of the results tab row, where this result's controls are shown while it is in front. */
    toolsHost: HTMLElement | undefined;
    /** This result is the one in front. */
    active: boolean;
  }

  let { workspace, tab, result, toolsHost, active }: Props = $props();



  // ---------- sorting and filtering ----------

  let filter = $state('');
  let sort = $state<{ column: number; descending: boolean } | null>(null);
  /** Rows visible after sorting and filtering; null until a view is applied. */
  let visibleRows = $state<number | null>(null);
  let viewVersion = $state(0);
  let note = $state<string | null>(null);
  let contextMenu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null);
  let grid = $state<ReturnType<typeof Grid> | undefined>();

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

  function setSort(next: { column: number; descending: boolean } | null) {
    if (!result.done) return;
    if (pending > 0) {
      note = 'Apply or discard the pending changes before sorting.';
      return;
    }
    sort = next;
    void applyView();
  }

  function sortBy(column: number) {
    // Ascending, then descending, then back to the order the rows arrived in.
    if (sort?.column !== column) setSort({ column, descending: false });
    else if (!sort.descending) setSort({ column, descending: true });
    else setSort(null);
  }

  // ---------- selecting and copying ----------

  let selection = $state<CellRange | null>(null);
  let focusCell = $state<CellPosition | null>(null);
  /** Totals of the selection; null for one cell or while they are being worked out. */
  let stats = $state<RangeStats | null>(null);
  let statsTimer: ReturnType<typeof setTimeout> | undefined;
  let statsRequest = 0;

  const selectedRows = $derived(selection ? Math.min(selection.rowEnd, rows - 1) - selection.rowStart + 1 : 0);
  const selectedColumns = $derived(selection ? selection.colEnd - selection.colStart + 1 : 0);

  function selectionChanged(range: CellRange | null, cell: CellPosition | null) {
    selection = range;
    focusCell = cell;
    stats = null;
    clearTimeout(statsTimer);
    const request = ++statsRequest;
    if (!range) return;
    const cells = (range.rowEnd - range.rowStart + 1) * (range.colEnd - range.colStart + 1);
    // One cell needs no totals. Beyond a point they would take longer than they are worth.
    if (cells < 2 || cells > 20_000_000) return;
    statsTimer = setTimeout(async () => {
      try {
        const found = await backend.rangeStats(result.id, range);
        if (request === statsRequest) stats = found;
      } catch {
        // The result was closed meanwhile.
      }
    }, 150);
  }

  function formatNumber(n: number): string {
    return n.toLocaleString('en-US', { maximumFractionDigits: 4 });
  }

  function tableName(): string {
    return exportTable.trim() || target?.table || 'table_name';
  }

  async function writeClipboard(text: string): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      note = 'The clipboard could not be written to.';
      return false;
    }
  }

  /** Copies the selected cells. The text is built by the backend from the stored rows. */
  async function copy(format: CopyFormat, headers = false) {
    const range = selection;
    if (!range) return;
    try {
      const text = await backend.copyRange(result.id, range, format, headers, tableName());
      if (!(await writeClipboard(text))) return;
      const cells = selectedRows * selectedColumns;
      note =
        cells === 1
          ? 'Copied 1 cell'
          : `Copied ${formatCount(selectedRows)} ${selectedRows === 1 ? 'row' : 'rows'} \u00d7 ${formatCount(selectedColumns)} ${selectedColumns === 1 ? 'column' : 'columns'}`;
    } catch (e) {
      note = asUiError(e).message;
    }
  }

  async function filterByCell(row: number, column: number) {
    try {
      const value = await backend.getCell(result.id, row, column);
      if (value.text === null || value.text.trim() === '') {
        note = 'An empty or NULL value cannot be filtered by.';
        return;
      }
      filter = value.text.slice(0, 200);
      await applyView();
    } catch (e) {
      note = asUiError(e).message;
    }
  }

  // ---------- export ----------

  let format = $state<ExportFormat>('csv');
  let exportTable = $state('');
  let exporting = $state(false);

  /** Open while the user chooses between every row the query returns and the rows held here. */
  let exportChoice = $state(false);
  /** Rows written so far by an export of the whole query; null when none is running. */
  let exportProgress = $state<number | null>(null);
  /** The connection the export of the whole query runs in, to stop it by. */
  let exportTab: string | null = null;

  function exportRows() {
    // A result that stopped at the row limit is not what the query returns.
    // Exporting it without asking is how a file of a thousand rows gets
    // taken for a table of six million.
    if (result.truncated) exportChoice = true;
    else void exportShown();
  }

  /** Writes the rows held here: what the grid shows, in its order, with its filter. */
  async function exportShown() {
    exportChoice = false;
    exporting = true;
    try {
      // What the grid shows at this moment: the file must hold exactly these rows.
      const shown = rows;
      const done = await backend.exportResult(result.id, format, tableName());
      if (done) {
        if (done.rows !== shown) {
          // Never expected. If it happens the file is not what is on screen, and that must not pass quietly.
          note = `WARNING: ${formatCount(done.rows)} rows were written to ${done.path}, but ${formatCount(shown)} are shown. The file is incomplete. Please report this.`;
        } else {
          note =
            (done.truncated
              ? `Exported only the first ${formatCount(done.rows)} rows`
              : done.filtered
                ? `Exported the ${formatCount(done.rows)} rows that match the filter`
                : `Exported all ${formatCount(done.rows)} rows`) +
            ` to ${done.path}` +
            (done.truncated ? '. The query returns more: this result stopped at the row limit' : '');
        }
      }
    } catch (e) {
      note = asUiError(e).message;
    } finally {
      exporting = false;
    }
  }

  /** Runs the query again and writes every row straight to the file, whatever the row limit. */
  async function exportAll() {
    exportChoice = false;
    exporting = true;
    exportProgress = 0;
    exportTab = `export-${crypto.randomUUID()}`;
    note = null;
    try {
      const done = await backend.exportQuery(workspace.info.id, exportTab, result.sql, format, tableName(), (rows) => {
        exportProgress = rows;
      });
      if (done) note = `Exported all ${formatCount(done.rows)} rows to ${done.path}`;
    } catch (e) {
      note = asUiError(e).message;
    } finally {
      exporting = false;
      exportProgress = null;
      exportTab = null;
    }
  }

  function stopExport() {
    if (exportTab) void backend.cancel(workspace.info.id, exportTab);
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

  /** Marks the rows for deletion, or unmarks them if every one already is. */
  function toggleDelete(first: number, last = first) {
    if (last - first >= 10_000) {
      note = 'That is more rows than can be deleted from the grid at once. Use a DELETE statement.';
      return;
    }
    const marked = new Set(deletes);
    let all = true;
    for (let row = first; row <= last; row++) all &&= marked.has(row);
    for (let row = first; row <= last; row++) {
      if (all) marked.delete(row);
      else marked.add(row);
    }
    deletes = [...marked];
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

  /** The menu for the selection; `row` and `column` are the cell that was right-clicked. */
  function cellMenu(row: number, column: number): MenuItem[] {
    const range = selection ?? { rowStart: row, rowEnd: row, colStart: column, colEnd: column };
    const manyRows = range.rowEnd > range.rowStart;
    const items: MenuItem[] = [
      { label: 'Copy', shortcut: 'Ctrl+C', action: () => void copy('tsv') },
      { label: 'Copy with Column Names', shortcut: 'Ctrl+Shift+C', action: () => void copy('tsv', true) },
      { label: 'Copy as CSV', action: () => void copy('csv', true) },
      { label: 'Copy as SQL INSERT', action: () => void copy('sql') },
      { label: 'Copy as IN (…) List', action: () => void copy('list') },
      separator,
      { label: manyRows ? 'Select Rows' : 'Select Row', shortcut: 'Shift+Space', action: () => grid?.selectRows(range.rowStart, range.rowEnd) },
      { label: range.colEnd > range.colStart ? 'Select Columns' : 'Select Column', shortcut: 'Ctrl+Space', action: () => grid?.selectColumns(range.colStart, range.colEnd) },
      { label: 'Select All', shortcut: 'Ctrl+A', action: () => grid?.selectAll() },
      separator,
      { label: 'Filter by This Value', disabled: !result.done || pending > 0, action: () => void filterByCell(row, column) },
      { label: 'View Value…', action: () => void showCell(row, column) },
    ];
    if (target) {
      const allMarked = deletes.includes(row);
      items.push(
        separator,
        { label: 'Edit Cell…', disabled: target.columns[column] === null || deletes.includes(row), action: () => void showCell(row, column) },
        {
          label: manyRows ? (allMarked ? 'Keep Rows' : `Delete ${formatCount(range.rowEnd - range.rowStart + 1)} Rows`) : allMarked ? 'Keep Row' : 'Delete Row',
          action: () => toggleDelete(range.rowStart, range.rowEnd),
        },
        { label: 'Add Row…', action: () => (newRow = {}) },
      );
    }
    return items;
  }

  function headerMenu(column: number): MenuItem[] {
    const sorted = sort?.column === column ? sort : null;
    return [
      { label: 'Sort Ascending', checked: !!sorted && !sorted.descending, disabled: !result.done, action: () => setSort({ column, descending: false }) },
      { label: 'Sort Descending', checked: !!sorted?.descending, disabled: !result.done, action: () => setSort({ column, descending: true }) },
      { label: 'Clear Sort', disabled: !sort, action: () => setSort(null) },
      separator,
      { label: 'Select Column', shortcut: 'Ctrl+Click', action: () => grid?.selectColumns(column, column) },
      { label: 'Copy Column Name', action: () => void writeClipboard(result.columns[column].name) },
      { label: 'Copy All Column Names', action: () => void writeClipboard(result.columns.map((c) => c.name).join(', ')) },
      separator,
      { label: 'Fit Column to Contents', shortcut: 'Double-click edge', action: () => grid?.autoSizeColumn(column) },
      { label: 'Fit All Columns', action: () => grid?.autoSizeAll() },
      // Also here, so a row can be added to a result that has none to right-click.
      ...(target ? [separator, { label: 'Add Row…', action: () => (newRow = {}) }] : []),
    ];
  }

  function closeMenu() {
    contextMenu = null;
    // Back to the grid, so Ctrl+C and the arrow keys act on the selection again.
    grid?.focus();
  }

  function modal(node: HTMLDialogElement) {
    node.showModal();
  }
</script>

<div class="result">
  <div class="bar" use:placeIn={toolsHost} hidden={!active}>
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
    {#if exportProgress !== null}
      <button class="danger" onclick={stopExport} title="Stop the export; the incomplete file is removed">&#9632; Stop Export</button>
    {:else}
      <button
        disabled={!result.done || exporting}
        onclick={exportRows}
        title={result.truncated
          ? 'Save to a file. This result stopped at the row limit; you will be asked whether to export every row the query returns.'
          : 'Save the rows to a file'}
      >
        {exporting ? 'Exporting…' : 'Export…'}
      </button>
    {/if}
    <button disabled={!selection} onclick={() => void copy('tsv')} title="Copy the selected cells (Ctrl+C). Right-click the grid for CSV, SQL INSERT and other forms.">
      Copy
    </button>
    {#if target && pending > 0}
      <span class="divider"></span>
      <span class="pending">{pending} pending {pending === 1 ? 'change' : 'changes'}</span>
      <button class="primary" onclick={openReview}>Review and Apply&hellip;</button>
      <button onclick={discard}>Discard</button>
    {/if}
  </div>

  <div class="grid-area">
    <Grid
      bind:this={grid}
      {result}
      {rows}
      {viewVersion}
      {sort}
      {overlay}
      onactivate={showCell}
      onselection={selectionChanged}
      oncopy={(headers) => void copy('tsv', headers)}
      onsort={sortBy}
      oncontext={(row, column, x, y) => (contextMenu = { items: cellMenu(row, column), x, y })}
      onheadercontext={(column, x, y) => (contextMenu = { items: headerMenu(column), x, y })}
    />
  </div>

  <!-- Shown in the bottom row of the window while this result is in front. -->
  <div class="status-items" use:placeIn={workspace.statusHost} hidden={!active || tab.id !== workspace.activeTabId}>
    {#if note}<span class="note" role="status" title={note}>{note}</span>{/if}
    {#if exportProgress !== null}
      <span class="note" role="status">Exporting every row: {formatCount(exportProgress)} written&hellip;</span>
    {/if}
    {#if selection && selectedRows * selectedColumns > 1}
      <span>
        {formatCount(selectedRows)} {selectedRows === 1 ? 'row' : 'rows'} &times; {formatCount(selectedColumns)}
        {selectedColumns === 1 ? 'column' : 'columns'}
      </span>
      {#if stats && stats.numbers > 0 && stats.sum !== null && stats.average !== null && stats.min !== null && stats.max !== null}
        <span title={`${formatCount(stats.numbers)} of the ${formatCount(stats.cells)} selected cells hold a number`}>
          Sum {formatNumber(stats.sum)}
        </span>
        <span>Avg {formatNumber(stats.average)}</span>
        <span>Min {formatNumber(stats.min)}</span>
        <span>Max {formatNumber(stats.max)}</span>
      {/if}
    {:else if focusCell}
      <span class="muted">Row {formatCount(focusCell.row + 1)}, {result.columns[focusCell.column]?.name}</span>
    {/if}
    {#if result.truncated}
      <span class="limit" title="More rows exist. Raise the limit and run again to see them, or use Export to write every row to a file.">
        Row limit reached
      </span>
    {/if}
    {#if result.elapsedMs !== null}<span class="muted" title="How long the statement took">{formatDuration(result.elapsedMs)}</span>{/if}
    <span class="count" title={whyNot ? `This result cannot be edited: ${whyNot}` : undefined}>
      {#if !result.done}
        <span class="state loading" role="status" aria-label="Loading rows" title="Rows are still arriving"></span>
      {:else if result.truncated}
        <span class="state partial" role="img" aria-label="Stopped at the row limit" title="Complete up to the row limit; more rows exist">&#9888;</span>
      {:else}
        <span class="state complete" role="img" aria-label="Complete" title="Every row has arrived">&#10003;</span>
      {/if}
      {#if visibleRows !== null && visibleRows !== result.rows}
        {formatCount(visibleRows)} of {formatCount(result.rows)} rows shown
      {:else}
        {formatCount(result.rows)} {result.rows === 1 ? 'row' : 'rows'}{result.done ? '' : ', still loading…'}
      {/if}
    </span>
  </div>
</div>

{#if exportChoice}
  <dialog use:modal aria-labelledby="export-title" onclose={() => (exportChoice = false)}>
    <div class="choice">
      <h2 id="export-title">Export which rows?</h2>
      <p>
        This result stopped at the row limit. It holds <strong>{formatCount(result.rows)}</strong>
        {result.rows === 1 ? 'row' : 'rows'}; the query returns more.
      </p>
      <button class="option primary" onclick={() => void exportAll()}>
        <strong>Every row the query returns</strong>
        <span>Runs the query again and writes each row straight to the file. Works for millions of rows. The filter and sorting here do not apply.</span>
      </button>
      <button class="option" onclick={() => void exportShown()}>
        <strong>Only the {formatCount(rows)} {rows === 1 ? 'row' : 'rows'} shown</strong>
        <span>What the grid shows now, in its order.</span>
      </button>
      <p class="muted why">
        The query has to run again because the row limit stops it on the server: the rows past the limit were never sent, so there is
        nothing here to save. To have every row ready after one run, set the row limit to <strong>No limit</strong> before running.
        All the rows are then fetched in the background, those that do not fit in memory are kept in a temporary file that is
        removed when the result closes, and Export writes them without running anything again.
      </p>
      <div class="buttons">
        <span class="spacer"></span>
        <button onclick={() => (exportChoice = false)}>Cancel</button>
      </div>
    </div>
  </dialog>
{/if}

{#if contextMenu}
  <Menu items={contextMenu.items} label="Grid actions" x={contextMenu.x} y={contextMenu.y} onclose={closeMenu} />
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

  /* Shown at the end of the results tab row (see `placeIn`), not here. */
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    white-space: nowrap;
  }

  .bar button,
  .bar select,
  .bar input {
    padding: 2px 8px;
  }

  .filter {
    width: 160px;
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

  /* In the window's bottom row (see `placeIn`). */
  .status-items {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .status-items > span {
    flex: none;
  }

  .count {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
  }

  .choice {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px 20px;
    width: 500px;
    max-width: 100%;
  }

  .choice p {
    margin: 0;
  }

  .option {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    padding: 10px 12px;
    border-radius: 8px;
    text-align: left;
    white-space: normal;
    font-weight: 400;
  }

  .option strong {
    font-size: 14px;
  }

  .choice .why {
    font-size: 12px;
  }

  .state {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    font-size: 13px;
    font-weight: 700;
    line-height: 1;
  }

  .state.complete {
    border-radius: 50%;
    background: var(--ok);
    color: var(--bg);
    font-size: 10px;
  }

  .state.partial {
    color: var(--warn-border);
    font-size: 15px;
  }

  .state.loading {
    border: 2px solid var(--placeholder);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: turn 0.8s linear infinite;
  }

  @keyframes turn {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .state.loading {
      animation-duration: 3s;
    }
  }

  .limit {
    color: var(--warn-text);
    background: var(--warn-bg);
    border: 1px solid var(--warn-border);
    border-radius: 4px;
    padding: 0 6px;
  }

  /* A notice (exported, copied, an error) is the one part that may be long:
     it gives way first, and its full text is in its tooltip. */
  .status-items > .note {
    flex: 0 1 auto;
    min-width: 60px;
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
