<script lang="ts">
  // Draws one chart. The chart library is loaded the first time this is shown.

  import { onMount } from 'svelte';
  import { luminance } from '../lib/chart';
  import type { ChartData } from '../lib/chart';
  import { buildOption, type ChartLook, type ChartView } from '../lib/chartOption';
  import type { Chart } from '../lib/echarts';
  import { APPEARANCE_EVENT } from '../lib/settings.svelte';

  interface Props {
    data: ChartData;
    view: ChartView;
    /** What the chart is, for screen readers. */
    label: string;
    /** Which way dragging across the chart zooms: along X, both ways, or not at all. */
    zoom: 'x' | 'xy' | null;
    /** A range was dragged out. `y` is null when only X is zoomed. */
    onzoom: (x: [number, number], y: [number, number] | null) => void;
    /** Double-click: back to everything. */
    onreset: () => void;
  }

  let { data, view, label, zoom, onzoom, onreset }: Props = $props();

  let host: HTMLDivElement;
  let chart = $state.raw<Chart | null>(null);
  let failed = $state('');
  /** Bumped when the theme changes, to draw again in its colours. */
  let appearance = $state(0);
  /** The rectangle being dragged out, in pixels within the chart. */
  let band = $state<{ left: number; top: number; width: number; height: number } | null>(null);

  function hex(value: string, fallback: string): string {
    const v = value.trim();
    if (/^#[0-9a-f]{6}$/i.test(v)) return v;
    if (/^#[0-9a-f]{3}$/i.test(v)) return '#' + [...v.slice(1)].map((c) => c + c).join('');
    return fallback;
  }

  function look(): ChartLook {
    const css = getComputedStyle(document.documentElement);
    const bg = hex(css.getPropertyValue('--bg'), '#ffffff');
    return {
      text: hex(css.getPropertyValue('--text'), '#1c2026'),
      muted: hex(css.getPropertyValue('--muted'), '#626b78'),
      border: hex(css.getPropertyValue('--border'), '#d5d9df'),
      panel: hex(css.getPropertyValue('--panel'), '#f4f5f7'),
      bg,
      dark: luminance(bg) < 0.3,
      font: getComputedStyle(document.body).fontFamily,
    };
  }

  onMount(() => {
    let closed = false;
    let observer: ResizeObserver | undefined;
    import('../lib/echarts')
      .then(({ init }) => {
        if (closed) return;
        chart = init(host, null, { renderer: 'canvas' });
        observer = new ResizeObserver(() => chart?.resize());
        observer.observe(host);
      })
      .catch((e) => (failed = `The chart could not be drawn: ${e instanceof Error ? e.message : String(e)}`));
    const recolor = () => appearance++;
    window.addEventListener(APPEARANCE_EVENT, recolor);
    return () => {
      closed = true;
      observer?.disconnect();
      window.removeEventListener(APPEARANCE_EVENT, recolor);
      chart?.dispose();
    };
  });

  $effect(() => {
    void appearance;
    if (!chart) return;
    try {
      chart.setOption(buildOption(data, view, look()) as never, { notMerge: true });
      failed = '';
    } catch (e) {
      failed = `The chart could not be drawn: ${e instanceof Error ? e.message : String(e)}`;
    }
  });

  /** The chart as a PNG, at twice the size on screen so it stays sharp when pasted. */
  export function png(): Uint8Array | null {
    if (!chart) return null;
    const url = chart.getDataURL({ type: 'png', pixelRatio: 2, backgroundColor: look().bg });
    const text = atob(url.slice(url.indexOf(',') + 1));
    const bytes = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);
    return bytes;
  }

  function startBand(e: PointerEvent) {
    if (!zoom || !chart || e.button !== 0) return;
    const box = host.getBoundingClientRect();
    const start: [number, number] = [e.clientX - box.left, e.clientY - box.top];
    if (!chart.containPixel({ gridIndex: 0 }, start)) return;
    const live = chart;
    const mode = zoom;
    const corner = (ev: PointerEvent): [number, number] => [
      Math.min(box.width, Math.max(0, ev.clientX - box.left)),
      Math.min(box.height, Math.max(0, ev.clientY - box.top)),
    ];
    const move = (ev: PointerEvent) => {
      const [x, y] = corner(ev);
      if (Math.abs(x - start[0]) < 4) {
        band = null;
        return;
      }
      band =
        mode === 'x'
          ? { left: Math.min(x, start[0]), top: 0, width: Math.abs(x - start[0]), height: box.height }
          : { left: Math.min(x, start[0]), top: Math.min(y, start[1]), width: Math.abs(x - start[0]), height: Math.abs(y - start[1]) };
    };
    const stop = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      const dragged = band;
      band = null;
      if (!dragged || dragged.width < 6 || (mode === 'xy' && dragged.height < 6)) return;
      const end = corner(ev);
      const a = live.convertFromPixel({ gridIndex: 0 }, start) as number[] | undefined;
      const b = live.convertFromPixel({ gridIndex: 0 }, end) as number[] | undefined;
      if (!a || !b || ![...a, ...b].every(Number.isFinite)) return;
      onzoom([Math.min(a[0], b[0]), Math.max(a[0], b[0])], mode === 'xy' ? [Math.min(a[1], b[1]), Math.max(a[1], b[1])] : null);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="chart" class:zoomable={zoom !== null} role="img" aria-label={label} onpointerdown={startBand} ondblclick={() => zoom && onreset()}>
  <div class="canvas" bind:this={host}></div>
  {#if band}
    <div class="band" style:left={`${band.left}px`} style:top={`${band.top}px`} style:width={`${band.width}px`} style:height={`${band.height}px`}></div>
  {/if}
  {#if failed}<p class="error-text failed">{failed}</p>{/if}
</div>

<style>
  .chart {
    position: relative;
    flex: 1;
    min-height: 0;
    min-width: 0;
  }

  .zoomable {
    cursor: crosshair;
  }

  .canvas {
    position: absolute;
    inset: 0;
  }

  .band {
    position: absolute;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    border: 1px solid var(--accent);
    pointer-events: none;
  }

  .failed {
    position: absolute;
    inset: 12px;
    margin: 0;
  }

  .chart :global(.chart-tip) {
    max-width: 320px;
    font-size: 12px;
    line-height: 1.5;
  }

  .chart :global(.chart-tip-title) {
    font-weight: 600;
    margin-bottom: 2px;
    overflow-wrap: anywhere;
    white-space: normal;
  }

  .chart :global(.chart-tip-row) {
    display: flex;
    align-items: baseline;
    gap: 6px;
    white-space: nowrap;
  }

  .chart :global(.chart-tip-row span:not(.chart-tip-swatch)) {
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 180px;
  }

  .chart :global(.chart-tip-row b) {
    margin-left: auto;
    padding-left: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .chart :global(.chart-tip-swatch) {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex: none;
    align-self: center;
  }
</style>
