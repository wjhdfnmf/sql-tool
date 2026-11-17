<script lang="ts">
  // The graph builder: takes over the window from the query while it is open.
  // Columns of the result go on the X axis, the Y axis or the colour; the
  // backend works out what to draw from every row and this draws it.

  import { backend } from '../lib/backend';
  import { BUCKETS, ROLE_LABELS, SEQUENTIAL, STATISTIC_LABELS, formatNumber, formatTime, luminance, parseTime } from '../lib/chart';
  import type { ChartData, ChartKind, ChartRole, Statistic, TimeBucket } from '../lib/chart';
  import type { ChartView } from '../lib/chartOption';
  import type { GraphState, Zone } from '../lib/graph.svelte';
  import type { MenuItem } from '../lib/menu';
  import { asUiError, formatCount, formatDuration, type ColumnKind } from '../lib/types';
  import ChartCanvas from './ChartCanvas.svelte';
  import GraphFilter from './GraphFilter.svelte';
  import Menu from './Menu.svelte';
  import RangeSlider from './RangeSlider.svelte';

  interface Props {
    graph: GraphState;
    /** What is being charted, for the top row: the tab and result. */
    title: string;
    onclose: () => void;
  }

  let { graph, title, onclose }: Props = $props();

  let data = $state.raw<ChartData | null>(null);
  let error = $state('');
  let busy = $state(false);
  let elapsedMs = $state<number | null>(null);
  /** What saving or copying the picture came to. */
  let notice = $state('');
  let canvas = $state<ChartCanvas | undefined>();
  let menu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null);
  /** A column being dragged from the list, and the zone under the pointer. */
  let dragging = $state<{ column: number; x: number; y: number; over: string | null } | null>(null);

  const KINDS: { kind: ChartKind; label: string; hint: string }[] = [
    { kind: 'bar', label: 'Bar', hint: 'A bar for each value of X: a count of rows, or a sum or average of Y' },
    { kind: 'histogram', label: 'Histogram', hint: 'How the values of one number or time column are spread' },
    { kind: 'scatter', label: 'Scatter', hint: 'A point for each row: one number or time column against another' },
  ];
  const GLYPHS: Partial<Record<ColumnKind, string>> = { number: '#', date: '◷', text: 'Aa' };
  const KIND_NAMES: Partial<Record<ColumnKind, string>> = { number: 'number', date: 'date or time', text: 'text' };

  // ---- asking the backend ----

  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const request = graph.request;
    const resultId = graph.resultId;
    clearTimeout(timer);
    if (!graph.ready) {
      generation++;
      data = null;
      error = '';
      busy = false;
      return;
    }
    // A slider being dragged asks many times a second; wait for it to pause.
    timer = setTimeout(() => void load(resultId, request), 80);
    return () => clearTimeout(timer);
  });

  async function load(resultId: number, request: typeof graph.request) {
    const mine = ++generation;
    busy = true;
    const started = performance.now();
    try {
      const found = await backend.chartData(resultId, request);
      if (mine !== generation) return;
      data = found;
      error = '';
      elapsedMs = Math.round(performance.now() - started);
    } catch (e) {
      if (mine !== generation) return;
      data = null;
      error = asUiError(e).message;
    } finally {
      if (mine === generation) busy = false;
    }
  }

  // ---- names and how it is drawn ----

  const xName = $derived.by(() => {
    if (graph.xLabel.trim()) return graph.xLabel.trim();
    if (!graph.x) return '';
    const bucket = data?.x.type === 'bins' ? data.x.bucket : null;
    return bucket ? `${graph.name(graph.x.column)} by ${bucket}` : graph.name(graph.x.column);
  });

  const yName = $derived.by(() => {
    if (graph.yLabel.trim()) return graph.yLabel.trim();
    const name = graph.name(graph.y?.column);
    if (graph.kind === 'scatter') return name;
    if (graph.kind === 'histogram' || !graph.y) return 'Rows';
    if (graph.y.role !== 'continuous' || graph.statistic === 'count') return `Rows with a ${name}`;
    return `${STATISTIC_LABELS[graph.statistic]} of ${name}`;
  });

  const yTime = $derived(graph.kind === 'scatter' && graph.y?.role === 'time');
  const view = $derived<ChartView>({
    kind: graph.kind,
    stacked: graph.stacked,
    legend: graph.legend,
    title: graph.title.trim(),
    xName,
    yName,
    logY: graph.logY && !yTime,
    pointSize: graph.pointSize,
    yTime,
    xRange: data?.x.type === 'continuous' ? graph.xRange : null,
    yRange: data?.x.type === 'continuous' ? graph.yRange : null,
    slot: (name) => graph.slot(name),
  });

  const label = $derived(
    `${KINDS.find((k) => k.kind === graph.kind)?.label ?? ''} chart: ${yName}${xName ? ` against ${xName}` : ''}${graph.color !== null ? `, coloured by ${graph.name(graph.color)}` : ''}`,
  );
  const zoom = $derived(data?.x.type === 'continuous' ? 'xy' : data?.x.type === 'bins' ? 'x' : null);
  const grouped = $derived((data?.series.length ?? 0) > 1);
  const empty = $derived(data !== null && data.rowsDrawn === 0);

  // ---- the range of X ----

  const extent = $derived(graph.x && graph.x.role !== 'discrete' ? (data?.xExtent ?? null) : null);
  const xTime = $derived(graph.x?.role === 'time');
  const low = $derived(extent ? Math.min(extent[1], Math.max(extent[0], graph.xRange?.[0] ?? extent[0])) : 0);
  const high = $derived(extent ? Math.max(extent[0], Math.min(extent[1], graph.xRange?.[1] ?? extent[1])) : 0);
  const zoomed = $derived(graph.xRange !== null || graph.yRange !== null);
  const showX = (value: number) => (xTime ? formatTime(value, data?.xTimeOnly) : formatNumber(value));

  function setXRange(from: number, to: number) {
    if (!extent) return;
    graph.xRange = from <= extent[0] && to >= extent[1] ? null : [from, to];
  }

  function typedX(which: 0 | 1, text: string) {
    if (!extent) return;
    const value = xTime ? parseTime(text) : Number(text.replaceAll(',', ''));
    if (value === null || !Number.isFinite(value) || text.trim() === '') return;
    if (which === 0) setXRange(Math.min(value, high), high);
    else setXRange(low, Math.max(value, low));
  }

  const HOUR = 3_600_000;
  const RECENT: [string, number][] = [
    ['Last hour', HOUR],
    ['Last 24 hours', 24 * HOUR],
    ['Last 7 days', 7 * 24 * HOUR],
    ['Last 30 days', 30 * 24 * HOUR],
    ['Last 90 days', 90 * 24 * HOUR],
    ['Last 365 days', 365 * 24 * HOUR],
  ];
  /** The stretches of time that are shorter than what the data spans. */
  const recent = $derived(extent && xTime && !data?.xTimeOnly ? RECENT.filter(([, ms]) => ms < extent[1] - extent[0]) : []);

  function resetZoom() {
    graph.xRange = null;
    graph.yRange = null;
  }

  // ---- putting columns in place ----

  function drop(zone: string, column: number) {
    if (zone === 'filter') graph.addFilter(column);
    else graph.put(zone as Zone, column);
  }

  function columnMenu(e: MouseEvent, column: number) {
    const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: box.left + 12,
      y: box.bottom,
      items: [
        { label: 'Put on the X axis', action: () => graph.put('x', column) },
        { label: 'Put on the Y axis', disabled: graph.kind === 'histogram', action: () => graph.put('y', column) },
        { label: 'Colour by', action: () => graph.put('color', column) },
        { separator: true },
        { label: 'Filter by', disabled: graph.filters.some((f) => f.column === column), action: () => graph.addFilter(column) },
      ],
    };
  }

  const zoneAt = (e: PointerEvent) => document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>('[data-zone]')?.dataset.zone ?? null;
  /** Set when a drag has just ended, so the click that follows it does not also open the menu. */
  let dragged = false;

  function startDrag(e: PointerEvent, column: number) {
    if (e.button !== 0) return;
    const [startX, startY] = [e.clientX, e.clientY];
    let moved = false;
    const move = (ev: PointerEvent) => {
      if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
      moved = true;
      dragging = { column, x: ev.clientX, y: ev.clientY, over: zoneAt(ev) };
    };
    const stop = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      dragging = null;
      if (!moved) return;
      dragged = true;
      setTimeout(() => (dragged = false));
      const zone = ev.type === 'pointerup' ? zoneAt(ev) : null;
      if (zone) drop(zone, column);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  }

  function choose(zone: Zone, value: string) {
    if (value === '') graph.clear(zone);
    else graph.put(zone, Number(value));
  }

  // ---- the picture ----

  async function saveImage() {
    const png = canvas?.png();
    if (!png) return;
    try {
      const path = await backend.chartSaveImage(png);
      if (path) notice = `Saved ${path}`;
    } catch (e) {
      notice = asUiError(e).message;
    }
  }

  async function copyImage() {
    const png = canvas?.png();
    if (!png) return;
    try {
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': new Blob([png.buffer as ArrayBuffer], { type: 'image/png' }) })]);
      notice = 'Picture copied.';
    } catch {
      notice = 'A picture cannot be copied from here on this system. Use Save image.';
    }
  }

  /** Whether the theme in use is a dark one: the shading of a dense plot runs the other way on dark. */
  function darkTheme(): boolean {
    const bg = getComputedStyle(document.documentElement).getPropertyValue('--bg').trim();
    return /^#[0-9a-f]{6}$/i.test(bg) && luminance(bg) < 0.3;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape' || e.defaultPrevented) return;
    // In a field or a list, Esc belongs to that control (closing its list, undoing what was typed).
    const at = e.target;
    if (at instanceof HTMLInputElement || at instanceof HTMLSelectElement || at instanceof HTMLTextAreaElement) return;
    if (menu) menu = null;
    else onclose();
  }
</script>

<svelte:window {onkeydown} />

{#snippet zone(which: Zone, heading: string, disabled: boolean)}
  {@const chosen = which === 'color' ? graph.color : (graph[which]?.column ?? null)}
  {@const axis = which === 'color' ? null : graph[which]}
  <div class="zone" class:over={dragging?.over === which} class:filled={chosen !== null && !disabled} data-zone={disabled ? undefined : which}>
    <span class="zone-name">{heading}</span>
    <div class="zone-row">
      <select
        aria-label={`Column for ${heading}`}
        title={disabled ? 'A histogram counts the rows in each bin, so it has no use for a Y column' : undefined}
        {disabled}
        value={disabled || chosen === null ? '' : String(chosen)}
        onchange={(e) => choose(which, e.currentTarget.value)}
      >
        <option value="">{disabled ? 'Not used' : 'Choose or drop a column'}</option>
        {#each graph.chartable as { index, column } (index)}<option value={String(index)}>{column.name}</option>{/each}
      </select>
      {#if chosen !== null && !disabled}
        <button class="quiet" aria-label={`Take ${graph.name(chosen)} off ${heading}`} title="Remove" onclick={() => graph.clear(which)}>&#215;</button>
      {/if}
    </div>
    {#if axis && which !== 'color' && !disabled}
      {@const roles = graph.rolesOf(axis.column)}
      <div class="zone-row">
        {#if roles.length > 1}
          <label>
            <span class="muted">as</span>
            <select value={axis.role} onchange={(e) => graph.setRole(which, e.currentTarget.value as ChartRole)}>
              {#each roles as role}<option value={role}>{ROLE_LABELS[role]}</option>{/each}
            </select>
          </label>
        {/if}
        {#if which === 'x' && graph.kind !== 'scatter' && axis.role === 'time'}
          <label>
            <span class="muted">by</span>
            <select value={graph.bucket ?? ''} onchange={(e) => (graph.bucket = (e.currentTarget.value || null) as TimeBucket | null)}>
              <option value="">auto</option>
              {#each BUCKETS as unit}<option value={unit}>{unit}</option>{/each}
            </select>
          </label>
        {:else if which === 'x' && graph.kind !== 'scatter' && axis.role === 'continuous'}
          <label title="About how many bars the numbers are cut into. Leave empty to have it chosen.">
            <span class="muted">bins</span>
            <input
              type="number"
              min="1"
              max="500"
              placeholder="auto"
              value={graph.bins ?? ''}
              onchange={(e) => (graph.bins = e.currentTarget.value === '' ? null : Math.min(500, Math.max(1, Math.round(Number(e.currentTarget.value)))))}
            />
          </label>
        {:else if which === 'y' && graph.kind === 'bar' && axis.role === 'continuous'}
          <label>
            <span class="muted">show</span>
            <select value={graph.statistic} onchange={(e) => (graph.statistic = e.currentTarget.value as Statistic)}>
              {#each Object.entries(STATISTIC_LABELS) as [value, text]}<option {value}>{text}</option>{/each}
            </select>
          </label>
        {/if}
      </div>
    {/if}
  </div>
{/snippet}

<div class="graph" style:--ramp={SEQUENTIAL.join(', ')}>
  <div class="top" role="toolbar" aria-label="Graph">
    <button onclick={onclose} title="Back to the query and its results (Esc)">&#8592; Back to query</button>
    <span class="divider"></span>
    <div class="kinds" role="group" aria-label="Kind of chart">
      {#each KINDS as k (k.kind)}
        <button aria-pressed={graph.kind === k.kind} title={k.hint} onclick={() => (graph.kind = k.kind)}>{k.label}</button>
      {/each}
    </div>
    <span class="what muted" title={title}>{title}</span>
    <button disabled={!data || empty} title="Copy a picture of the chart" onclick={copyImage}>Copy image</button>
    <button disabled={!data || empty} title="Save a picture of the chart as a PNG file" onclick={saveImage}>Save image</button>
  </div>

  <div class="body">
    <div class="side" aria-label="Columns and axes">
      <h3>Columns</h3>
      <div class="columns">
        {#each graph.chartable as { index, column } (index)}
          <button
            class="quiet column"
            title={`${column.name} (${KIND_NAMES[column.kind]}). Drag onto an axis, or click to choose where it goes.`}
            aria-haspopup="menu"
            onpointerdown={(e) => startDrag(e, index)}
            onclick={(e) => !dragged && columnMenu(e, index)}
          >
            <span class="glyph" aria-hidden="true">{GLYPHS[column.kind]}</span>
            <span class="column-name">{column.name}</span>
          </button>
        {/each}
      </div>
      {@render zone('x', 'X axis', false)}
      {@render zone('y', 'Y axis', graph.kind === 'histogram')}
      {@render zone('color', 'Colour by', false)}
    </div>

    <div class="stage">
      {#if error}
        <div class="message"><p class="error-text">{error}</p></div>
      {:else if !graph.ready}
        <div class="message"><p class="muted">{graph.hint}</p></div>
      {:else if data && !empty}
        <ChartCanvas bind:this={canvas} {data} {view} {label} {zoom} onzoom={(x, y) => ((graph.xRange = x), y && (graph.yRange = y))} onreset={resetZoom} />
      {:else if data}
        <div class="message">
          <p class="muted">
            {data.rowsFiltered === 0 ? 'No rows pass the filters.' : 'None of the rows has a value that can be drawn on these axes.'}
            {#if zoomed}<button class="link" onclick={resetZoom}>Show the whole range</button>{/if}
          </p>
        </div>
      {:else}
        <div class="message"><p class="muted">Reading the columns…</p></div>
      {/if}

      {#if extent && extent[1] > extent[0]}
        <div class="x-range">
          <span class="muted">{xTime ? 'Time range' : `Range of ${graph.name(graph.x?.column)}`}</span>
          <input type="text" aria-label="From" value={showX(low)} readonly={data?.xTimeOnly} onchange={(e) => typedX(0, e.currentTarget.value)} />
          <div class="slider">
            <RangeSlider min={extent[0]} max={extent[1]} {low} {high} label={xTime ? 'Time range' : 'Range of X'} format={showX} onchange={setXRange} />
          </div>
          <input type="text" aria-label="To" value={showX(high)} readonly={data?.xTimeOnly} onchange={(e) => typedX(1, e.currentTarget.value)} />
          {#if recent.length > 0}
            <select
              aria-label="Show a recent stretch of time"
              value=""
              onchange={(e) => {
                const ms = Number(e.currentTarget.value);
                if (ms > 0) setXRange(extent[1] - ms, extent[1]);
                e.currentTarget.value = '';
              }}
            >
              <option value="">Recent…</option>
              {#each recent as [text, ms]}<option value={String(ms)}>{text}</option>{/each}
            </select>
          {/if}
          <button disabled={!zoomed} title="Show the whole range again (or double-click the chart)" onclick={resetZoom}>All</button>
        </div>
      {/if}
    </div>

    <div class="side right" aria-label="Filters and options">
      <h3>Filters</h3>
      <div class="filters" class:over={dragging?.over === 'filter'} data-zone="filter">
        {#each graph.filters as filter (filter.column)}
          <GraphFilter {graph} {filter} onremove={() => graph.removeFilter(filter.column)} />
        {/each}
        <select
          aria-label="Add a filter"
          value=""
          onchange={(e) => {
            if (e.currentTarget.value !== '') graph.addFilter(Number(e.currentTarget.value));
            e.currentTarget.value = '';
          }}
        >
          <option value="">Add a filter…</option>
          {#each graph.chartable.filter(({ index }) => !graph.filters.some((f) => f.column === index)) as { index, column } (index)}
            <option value={String(index)}>{column.name}</option>
          {/each}
        </select>
      </div>

      <h3>Options</h3>
      <div class="options">
        {#if graph.kind === 'bar' && graph.x?.role === 'discrete'}
          <label>
            <span>Order</span>
            <select bind:value={graph.sort}>
              <option value="value">Largest first</option>
              <option value="label">By name</option>
              <option value="seen">As in the result</option>
            </select>
          </label>
          <label title="More values than this are merged into one bar called Other">
            <span>Bars shown</span>
            <input type="number" min="1" max="500" value={graph.maxCategories} onchange={(e) => (graph.maxCategories = Math.min(500, Math.max(1, Math.round(Number(e.currentTarget.value) || 40))))} />
          </label>
        {/if}
        {#if graph.kind !== 'scatter' && graph.color !== null}
          <label class="check"><input type="checkbox" bind:checked={graph.stacked} disabled={graph.logY} /> <span>Stack the colours</span></label>
        {/if}
        {#if graph.kind === 'scatter'}
          <label title="A plot of more than 50,000 points is drawn as shading, or as a sample of the points">
            <span>Many points</span>
            <select bind:value={graph.large}>
              <option value="density">Shade by count</option>
              <option value="sample">Random sample</option>
            </select>
          </label>
          <label>
            <span>Point size</span>
            <select value={String(graph.pointSize)} onchange={(e) => (graph.pointSize = Number(e.currentTarget.value))}>
              <option value="0">Auto</option>
              {#each [2, 3, 4, 6, 8, 10] as size}<option value={String(size)}>{size}</option>{/each}
            </select>
          </label>
        {/if}
        {#if !yTime}
          <label class="check" title="Each step up the Y axis is ten times the last. Values of zero or less are not drawn.">
            <input type="checkbox" bind:checked={graph.logY} /> <span>Log scale on Y</span>
          </label>
        {/if}
        {#if graph.color !== null}
          <label>
            <span>Legend</span>
            <select bind:value={graph.legend}>
              <option value="right">On the right</option>
              <option value="bottom">Underneath</option>
              <option value="hidden">Hidden</option>
            </select>
          </label>
        {/if}
        <label><span>Title</span><input type="text" bind:value={graph.title} placeholder="None" /></label>
        <label><span>X label</span><input type="text" bind:value={graph.xLabel} placeholder={graph.name(graph.x?.column) || 'Automatic'} /></label>
        <label><span>Y label</span><input type="text" bind:value={graph.yLabel} placeholder="Automatic" /></label>
      </div>
    </div>
  </div>

  <div class="foot" role="status">
    {#if busy}<span class="working" aria-hidden="true"></span>{/if}
    {#if data}
      <span>
        {formatCount(data.rowsFiltered)} of {formatCount(data.rowsTotal)} rows{data.rowsFiltered < data.rowsTotal ? ' pass the filters and range' : ''}
      </span>
      {#if data.rowsDrawn < data.rowsFiltered}
        <span>{formatCount(data.rowsFiltered - data.rowsDrawn)} of them have no value to draw</span>
      {/if}
      {#if data.density}
        <span class="key" title="How many rows fall in one cell of the plot">
          1 row <span class="ramp" class:dark={darkTheme()} aria-hidden="true"></span> {formatCount(data.density.max)} rows
        </span>
      {/if}
      {#each data.notes as note}<span class="note" title={note}>{note}</span>{/each}
      {#if zoom}<span class="muted">Drag across the chart to zoom in</span>{/if}
    {:else if busy}
      <span>Working…</span>
    {/if}
    <span class="spacer"></span>
    {#if notice}<span class="note" title={notice}>{notice}</span>{/if}
    {#if data && elapsedMs !== null}<span class="muted">{formatDuration(elapsedMs)}</span>{/if}
  </div>
</div>

{#if dragging}
  <div class="ghost" style:left={`${dragging.x + 10}px`} style:top={`${dragging.y + 8}px`}>{graph.name(dragging.column)}</div>
{/if}

{#if menu}
  <Menu items={menu.items} x={menu.x} y={menu.y} label="Where the column goes" onclose={() => (menu = null)} />
{/if}

<style>
  .graph {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  .top,
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    background: var(--panel);
    flex: none;
  }

  .top {
    border-bottom: 1px solid var(--border);
  }

  .divider {
    width: 1px;
    align-self: stretch;
    background: var(--border);
    margin: 2px 4px;
  }

  .kinds {
    display: flex;
  }

  .kinds button {
    border-radius: 0;
    margin-left: -1px;
  }

  .kinds button:first-child {
    border-radius: 4px 0 0 4px;
    margin-left: 0;
  }

  .kinds button:last-child {
    border-radius: 0 4px 4px 0;
  }

  .kinds button[aria-pressed='true'] {
    background: var(--selection);
    border-color: var(--accent);
    position: relative;
    z-index: 1;
  }

  .what {
    flex: 1;
    min-width: 0;
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 8px;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 232px minmax(0, 1fr) 256px;
  }

  .side {
    min-height: 0;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--panel);
    border-right: 1px solid var(--border);
  }

  .side.right {
    border-right: none;
    border-left: 1px solid var(--border);
  }

  h3 {
    margin: 2px 0 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }

  .side.right h3:not(:first-child) {
    margin-top: 10px;
  }

  .columns {
    display: flex;
    flex-direction: column;
    flex: 0 1 auto;
    min-height: 60px;
    max-height: 40%;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    padding: 2px;
  }

  .column {
    display: flex;
    align-items: center;
    gap: 6px;
    text-align: left;
    padding: 2px 6px;
    cursor: grab;
    touch-action: none;
    flex: none;
  }

  .glyph {
    width: 18px;
    flex: none;
    text-align: center;
    color: var(--muted);
    font-size: 11px;
  }

  .column-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .zone {
    border: 1px dashed var(--border);
    border-radius: 6px;
    padding: 5px 7px 7px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: var(--bg);
    flex: none;
  }

  .zone.filled {
    border-style: solid;
  }

  .zone.over,
  .filters.over {
    border-color: var(--accent);
    background: var(--selection);
  }

  .zone-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }

  .zone-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .zone-row > select {
    flex: 1;
    min-width: 0;
  }

  .zone-row label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }

  .zone-row label select,
  .zone-row label input {
    padding: 1px 4px;
    font-size: 12px;
  }

  .zone-row input[type='number'] {
    width: 64px;
  }

  .zone-row button {
    padding: 0 6px;
  }

  .stage {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 8px 8px 4px;
  }

  .message {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    text-align: center;
  }

  .message p {
    max-width: 440px;
    margin: 0;
  }

  .x-range {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 2px 4px;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }

  .x-range .slider {
    flex: 1;
    min-width: 80px;
  }

  .x-range input {
    width: 132px;
    padding: 2px 5px;
    font-size: 12px;
  }

  .x-range select,
  .x-range button {
    padding: 2px 6px;
    font-size: 12px;
  }

  .filters {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border: 1px dashed transparent;
    border-radius: 6px;
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
  }

  .options label {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .options label > span:first-child {
    width: 78px;
    flex: none;
    color: var(--muted);
  }

  .options label.check > span {
    width: auto;
    color: var(--text);
  }

  .options select,
  .options input[type='text'],
  .options input[type='number'] {
    flex: 1;
    min-width: 0;
    padding: 2px 5px;
    font-size: 12px;
  }

  .foot {
    border-top: 1px solid var(--border);
    font-size: 12px;
    gap: 14px;
    min-height: 26px;
    white-space: nowrap;
    overflow: hidden;
  }

  .foot .note {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .spacer {
    flex: 1;
  }

  .key {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }

  .ramp {
    width: 72px;
    height: 9px;
    border-radius: 2px;
    border: 1px solid var(--border);
    background: linear-gradient(to right, var(--ramp));
  }

  .ramp.dark {
    background: linear-gradient(to left, var(--ramp));
  }

  .working {
    width: 10px;
    height: 10px;
    flex: none;
    border-radius: 50%;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    animation: turn 0.8s linear infinite;
  }

  @keyframes turn {
    to {
      transform: rotate(360deg);
    }
  }

  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
  }

  .ghost {
    position: fixed;
    z-index: 50;
    pointer-events: none;
    padding: 2px 8px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--bg);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
    font-size: 12px;
  }
</style>
