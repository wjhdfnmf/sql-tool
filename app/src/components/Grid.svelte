<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { GridView, type CellPosition, type GridOverlay } from '../lib/grid';
  import type { ResultTab } from '../lib/state.svelte';
  import type { CellRange } from '../lib/types';

  interface Props {
    result: ResultTab;
    /** Rows to show: the result's rows, or fewer while a filter is on. */
    rows: number;
    /** Changes whenever the rows change order or number, so the grid reloads. */
    viewVersion: number;
    sort: { column: number; descending: boolean } | null;
    overlay: GridOverlay;
    /** A cell was opened (double-click or Enter). */
    onactivate: (row: number, column: number) => void;
    /** The selected cells changed; `range` is null when nothing is selected. */
    onselection: (range: CellRange | null, focus: CellPosition | null) => void;
    /** Ctrl+C on the selection; `headers` when Shift was held too. */
    oncopy: (headers: boolean) => void;
    onsort: (column: number) => void;
    oncontext: (row: number, column: number, x: number, y: number) => void;
    onheadercontext: (column: number, x: number, y: number) => void;
  }

  let { result, rows, viewVersion, sort, overlay, onactivate, onselection, oncopy, onsort, oncontext, onheadercontext }: Props =
    $props();

  let canvas: HTMLCanvasElement;
  let scroller: HTMLDivElement;
  let spacer: HTMLDivElement;
  let grid = $state<GridView | undefined>();
  let seenVersion = 0;

  export function selectAll() {
    grid?.selectAll();
  }
  export function selectRows(first: number, last: number) {
    grid?.selectRows(first, last);
  }
  export function selectColumns(first: number, last: number) {
    grid?.selectColumns(first, last);
  }
  export function autoSizeColumn(column: number) {
    grid?.autoSizeColumn(column);
  }
  export function autoSizeAll() {
    grid?.autoSizeAll();
  }
  /** Gives the grid the keyboard again, after a menu or dialog had it. */
  export function focus() {
    grid?.focus();
  }

  onMount(() => {
    const view = new GridView(canvas, scroller, spacer, (offset, count) => backend.getRows(result.id, offset, count), {
      onActivate: (row, column) => onactivate(row, column),
      onSelection: (range, cell) => onselection(range, cell),
      onCopy: (headers) => oncopy(headers),
      onSort: (column) => onsort(column),
      onContext: (row, column, x, y) => oncontext(row, column, x, y),
      onHeaderContext: (column, x, y) => onheadercontext(column, x, y),
    });
    view.setColumns(result.columns);
    grid = view;
    return () => view.destroy();
  });

  $effect(() => {
    if (!grid) return;
    if (viewVersion !== seenVersion) {
      // Sorted or filtered: everything cached is for the old order.
      seenVersion = viewVersion;
      grid.resetRows(rows);
    } else {
      grid.setRowCount(rows);
    }
  });

  $effect(() => {
    grid?.setSort(sort);
  });

  $effect(() => {
    grid?.setOverlay(overlay);
  });
</script>

<div class="grid">
  <canvas bind:this={canvas}></canvas>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="scroller" bind:this={scroller} tabindex="0" role="grid" aria-label={`${result.title}, ${rows} rows`}>
    <div bind:this={spacer}></div>
  </div>
</div>

<style>
  .grid {
    position: relative;
    height: 100%;
    min-height: 0;
  }

  canvas {
    position: absolute;
    top: 0;
    left: 0;
  }

  /* Transparent layer over the canvas. It owns the native scrollbars and
     input; the canvas underneath paints whatever the scroll position maps to. */
  .scroller {
    position: absolute;
    inset: 0;
    overflow: auto;
  }

  .scroller > div {
    pointer-events: none;
  }
</style>
