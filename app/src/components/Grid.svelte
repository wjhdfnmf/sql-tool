<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { GridView, type GridOverlay } from '../lib/grid';
  import type { ResultTab } from '../lib/state.svelte';

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
    onselect: (row: number, column: number) => void;
    onsort: (column: number) => void;
    oncontext: (row: number, column: number, x: number, y: number) => void;
  }

  let { result, rows, viewVersion, sort, overlay, onactivate, onselect, onsort, oncontext }: Props = $props();

  let canvas: HTMLCanvasElement;
  let scroller: HTMLDivElement;
  let spacer: HTMLDivElement;
  let grid = $state<GridView | undefined>();
  let seenVersion = 0;

  async function copyCell(row: number, column: number) {
    try {
      const value = await backend.getCell(result.id, row, column);
      await navigator.clipboard.writeText(value.text ?? 'NULL');
    } catch {
      // Clipboard access can be refused; nothing useful to do about it here.
    }
  }

  onMount(() => {
    const view = new GridView(canvas, scroller, spacer, (offset, count) => backend.getRows(result.id, offset, count), {
      onActivate: (row, column) => onactivate(row, column),
      onSelect: (row, column) => onselect(row, column),
      onCopy: copyCell,
      onSort: (column) => onsort(column),
      onContext: (row, column, x, y) => oncontext(row, column, x, y),
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
