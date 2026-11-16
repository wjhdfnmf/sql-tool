<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { GridView } from '../lib/grid';
  import type { ResultTab } from '../lib/state.svelte';

  interface Props {
    result: ResultTab;
    /** A cell was opened (double-click or Enter). */
    onactivate: (row: number, column: number) => void;
    onselect: (row: number, column: number) => void;
  }

  let { result, onactivate, onselect }: Props = $props();

  let canvas: HTMLCanvasElement;
  let scroller: HTMLDivElement;
  let spacer: HTMLDivElement;
  let grid = $state<GridView | undefined>();

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
      onActivate: onactivate,
      onSelect: onselect,
      onCopy: copyCell,
    });
    view.setColumns(result.columns);
    grid = view;
    return () => view.destroy();
  });

  $effect(() => {
    grid?.setRowCount(result.rows);
  });
</script>

<div class="grid">
  <canvas bind:this={canvas}></canvas>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="scroller" bind:this={scroller} tabindex="0" role="grid" aria-label={`${result.title}, ${result.rows} rows`}>
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
