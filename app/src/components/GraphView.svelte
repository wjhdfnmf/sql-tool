<script lang="ts">
  // The graph builder: takes over the window from the query while it is open.
  // Columns of the result go on the X axis, the Y axis or the colour; the
  // backend works out what to draw from every row and this draws it.

  import { backend } from '../lib/backend';
  import {
    BUCKETS,
    KINDS,
    LINE_ROLE_NAMES,
    ROLE_LABELS,
    RUN_RULES,
    SEQUENTIAL,
    STATISTIC_LABELS,
    TREND_LABELS,
    capability,
    formatNumber,
    formatTime,
    luminance,
    parseTime,
  } from '../lib/chart';
  import type { ChartData, ChartKind, ChartRole, RefLine, Statistic, TimeBucket } from '../lib/chart';
  import { trendChoices, trendLines, xIsScale, type ChartView } from '../lib/chartOption';
  import type { GraphDialogRequest, GraphState, Zone } from '../lib/graph.svelte';
  import { separator, type MenuItem } from '../lib/menu';
  import { asUiError, formatCount, formatDuration, type ColumnKind } from '../lib/types';
  import ChartCanvas, { type ChartSpot } from './ChartCanvas.svelte';
  import GraphDialog from './GraphDialog.svelte';
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
  let menu = $state<{ items: MenuItem[]; x: number; y: number; label: string } | null>(null);
  let dialog = $state.raw<GraphDialogRequest | null>(null);
  /** A column being dragged from the list, and the zone under the pointer. */
  let dragging = $state<{ column: number; x: number; y: number; over: string | null } | null>(null);

  const GLYPHS: Partial<Record<ColumnKind, string>> = { number: '#', date: '◷', text: 'Aa' };
  const KIND_NAMES: Partial<Record<ColumnKind, string>> = { number: 'number', date: 'date or time', text: 'text' };
  const is = (...kinds: ChartKind[]) => kinds.includes(graph.kind);

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
    if (!graph.x) return data?.x.type === 'sequence' ? 'Row, in the order of the result' : '';
    const bucket = data?.x.type === 'bins' ? data.x.bucket : null;
    return bucket ? `${graph.name(graph.x.column)} by ${bucket}` : graph.name(graph.x.column);
  });

  const yName = $derived.by(() => {
    if (graph.yLabel.trim()) return graph.yLabel.trim();
    const name = graph.name(graph.y?.column);
    if (is('scatter', 'box')) return name;
    if (is('control')) return data?.control && !data.control.individuals ? `Average of ${name}` : name;
    if (is('histogram') || !graph.y) return 'Rows';
    if (graph.y.role !== 'continuous' || graph.statistic === 'count') return `Rows with a ${name}`;
    return `${STATISTIC_LABELS[graph.statistic]} of ${name}`;
  });

  const yTime = $derived(is('scatter', 'box') && graph.y?.role === 'time');
  /** X is a scale of numbers or time on this chart, so lines can be drawn at its values and it can be zoomed. */
  const scaleX = $derived(data !== null && xIsScale(data, graph.kind));
  const xTime = $derived(graph.x?.role === 'time');
  const logAllowed = $derived(is('bar', 'line', 'histogram', 'scatter') && !yTime);
  const view = $derived<ChartView>({
    kind: graph.kind,
    stacked: graph.stacked,
    legend: graph.legend,
    title: graph.title.trim(),
    xName,
    yName,
    logY: graph.logY && logAllowed,
    pointSize: graph.pointSize,
    xTime: xTime && scaleX,
    yTime,
    xRange: data?.x.type === 'continuous' ? graph.xRange : null,
    yRange: data?.x.type === 'continuous' ? graph.yRange : null,
    slot: (name) => graph.slot(name),
    lines: graph.shownLines,
    trend: graph.trend,
    normalCurve: graph.normalCurve,
    barLabels: graph.barLabels,
    paretoLine: graph.paretoLine,
    paretoCutoff: graph.paretoCutoff,
    paretoFade: graph.paretoFade,
    rules: graph.rules,
    zones: graph.zones,
    spreadChart: graph.spreadChart,
  });

  /** A number typed into an option, or null if the field was left empty or does not hold one. */
  const typedNumber = (text: string): number | null => {
    const value = Number(text.replaceAll(',', '').trim());
    return text.trim() !== '' && Number.isFinite(value) ? value : null;
  };

  function toggleRule(rule: number, on: boolean) {
    graph.rules = on ? [...graph.rules, rule].sort((a, b) => a - b) : graph.rules.filter((r) => r !== rule);
  }

  const label = $derived(
    `${KINDS.find((k) => k.kind === graph.kind)?.label ?? ''} chart: ${yName}${xName ? ` against ${xName}` : ''}${graph.color !== null ? `, coloured by ${graph.name(graph.color)}` : ''}`,
  );
  const zoom = $derived(data?.x.type === 'continuous' ? 'xy' : scaleX ? 'x' : null);
  const empty = $derived(data !== null && data.rowsDrawn === 0);
  const usesColor = $derived(!is('pie', 'pareto', 'box', 'control'));

  // ---- the range of X ----

  const extent = $derived(graph.x && graph.x.role !== 'discrete' ? (data?.xExtent ?? null) : null);
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

  // ---- lines across the chart, the trend line, and names ----

  const lineValue = (line: RefLine) => (line.time ? formatTime(line.value) : formatNumber(line.value));
  /** A line as it is on the chart in front, with its axis; undefined if it belongs to something this chart does not show. */
  const shown = (line: RefLine) => graph.shownLines.find((l) => l.id === line.id);
  const choices = $derived(trendChoices(data, graph.kind));
  const trends = $derived(data && graph.trend ? trendLines(data, view) : []);

  function lineDialog(line: RefLine, existing: boolean) {
    dialog = {
      type: 'line',
      line,
      axes: existing ? [line.axis] : scaleX ? ['x', 'y'] : ['y'],
      time: { x: xTime, y: yTime },
      measured: graph.measured,
      save: (saved) => graph.saveLine(saved),
      remove: existing ? () => graph.removeLine(line.id) : undefined,
    };
  }

  /** A new line at a value: where the pointer was, or failing that the average of the data. */
  function addLine(axis: 'x' | 'y', value: number | null) {
    const fallback = graph.measured === axis ? (data?.stats?.mean ?? 0) : axis === 'x' && extent ? (extent[0] + extent[1]) / 2 : 0;
    lineDialog(graph.newLine(axis, value ?? fallback), false);
  }

  function trendDialog() {
    dialog = {
      type: 'trend',
      setup: graph.trend,
      moving: choices.moving,
      perGroup: choices.perGroup,
      save: (setup) => (graph.trend = setup),
      remove: graph.trend ? () => (graph.trend = null) : undefined,
    };
  }

  function nameDialog(which: 'title' | 'x' | 'y') {
    dialog =
      which === 'title'
        ? { type: 'text', heading: 'Title of the chart', label: 'Title', value: graph.title, placeholder: 'None', save: (text) => (graph.title = text) }
        : which === 'x'
          ? { type: 'text', heading: 'Name of the X axis', label: 'X axis', value: graph.xLabel, placeholder: graph.name(graph.x?.column) || 'Automatic', save: (text) => (graph.xLabel = text) }
          : { type: 'text', heading: 'Name of the Y axis', label: 'Y axis', value: graph.yLabel, placeholder: 'Automatic', save: (text) => (graph.yLabel = text) };
  }

  /** The right-click menu of the chart: what it offers depends on where the click landed. */
  function chartMenu(spot: ChartSpot) {
    const items: MenuItem[] = [];
    const line = graph.shownLines.find((l) => l.id === spot.line);
    if (line) {
      const what = line.name ? `\u201c${line.name}\u201d` : `at ${lineValue(line)}`;
      items.push({ label: `Change the line ${what}…`, action: () => lineDialog(line, true) }, { label: `Remove the line ${what}`, action: () => graph.removeLine(line.id) }, separator);
    }
    const lineAt = (axis: 'x' | 'y', text: string): MenuItem => ({ label: text, action: () => addLine(axis, axis === 'x' ? spot.x : spot.y) });
    const logScale: MenuItem = { label: 'Log scale', checked: graph.logY, disabled: !logAllowed, action: () => (graph.logY = !graph.logY) };
    if (spot.area === 'x') {
      if (scaleX) items.push(lineAt('x', 'Add a line at this value…'));
      items.push({ label: 'Rename the X axis…', action: () => nameDialog('x') });
    } else if (spot.area === 'y') {
      items.push(lineAt('y', 'Add a line at this value…'), { label: 'Rename the Y axis…', action: () => nameDialog('y') }, logScale);
    } else {
      if (spot.area === 'plot') {
        items.push({ label: graph.trend ? 'Change the trend line…' : 'Trend line…', disabled: !choices.any, action: trendDialog });
        if (graph.trend) items.push({ label: 'Remove the trend line', action: () => (graph.trend = null) });
        items.push(separator, lineAt('y', 'Add a line across at this height…'));
        if (scaleX) items.push(lineAt('x', 'Add an upright line here…'));
        items.push(separator);
      }
      items.push({ label: 'Title…', action: () => nameDialog('title') });
      if (!is('pie')) items.push({ label: 'Rename the X axis…', action: () => nameDialog('x') }, { label: 'Rename the Y axis…', action: () => nameDialog('y') });
      if (zoomed) items.push(separator, { label: 'Show the whole range', action: resetZoom });
    }
    menu = { items, x: spot.clientX, y: spot.clientY, label: 'Chart' };
  }

  // ---- the figures beside the chart ----

  type Figure = [label: string, value: string, hint?: string];

  const figures = $derived.by((): Figure[] => {
    if (!data) return [];
    const out: Figure[] = [];
    const stats = data.stats;
    // Times are held as milliseconds; their average and spread as numbers would mean nothing.
    const measuredTime = graph.measured === 'x' ? xTime : graph.y?.role === 'time';
    if (stats && !measuredTime) {
      out.push(['Rows', formatCount(stats.rows)], ['Average', formatNumber(stats.mean)], ['Std dev', formatNumber(stats.stdDev), 'The standard deviation of all the values']);
      if (stats.sigmaWithin !== null) {
        out.push(['Short-term', formatNumber(stats.sigmaWithin), 'The standard deviation judged from the change between one row and the next. Smaller than Std dev when the level drifts.']);
      }
      out.push(['Smallest', formatNumber(stats.min)], ['Largest', formatNumber(stats.max)]);
    }
    const control = data.control;
    if (control) {
      const points = data.series[0]?.values.filter((v) => v !== null).length ?? 0;
      const given = control.fixed ? ' (as given)' : '';
      out.push(
        ['Centre line', formatNumber(control.center) + (graph.fixedCenter !== null ? given : '')],
        [
          'Sigma',
          formatNumber(control.sigma) + (graph.fixedSigma !== null ? given : ''),
          graph.fixedSigma !== null ? 'As given' : control.individuals ? 'From the average change between one point and the next' : 'From the spread within the points, pooled',
        ],
        ['Points', formatCount(points)],
      );
      // What each watched rule found, and how many points are marked in all.
      const watched = graph.rules.reduce((bits, rule) => bits | (1 << (rule - 1)), 0);
      for (const rule of RUN_RULES.filter((r) => graph.rules.includes(r.rule))) {
        const hits = control.signals.filter((signal) => signal & (1 << (rule.rule - 1))).length;
        out.push([`Rule ${rule.rule}`, hits === 0 ? 'none' : `${formatCount(hits)} ${hits === 1 ? 'point' : 'points'}`, rule.text]);
      }
      const marked = control.signals.filter((signal) => signal & watched).length;
      out.push(['Marked', `${formatCount(marked)} of ${formatCount(points)}`, 'Points that break at least one of the rules being watched']);
      const wide = control.spread.filter((v, i) => v !== null && control.spreadUpper[i] !== null && v > (control.spreadUpper[i] as number)).length;
      if (wide > 0) out.push(['Spread too wide', `${formatCount(wide)} ${wide === 1 ? 'point' : 'points'}`, 'Points whose spread is above its own upper limit, on the chart underneath']);
    }
    if (stats && !measuredTime && graph.measured) {
      const [lsl, usl, target] = [graph.spec('lsl'), graph.spec('usl'), graph.spec('target')];
      const fit = capability(stats, lsl, usl);
      if (fit) {
        const index = (value: number | null) => (value === null ? 'n/a' : value.toFixed(2));
        const share = (rows: number | null) => (rows === null ? '' : `${formatCount(rows)} (${((rows / stats.rows) * 100).toFixed(rows / stats.rows < 0.01 ? 3 : 1)}%)`);
        if (lsl !== null) out.push(['LSL', formatNumber(lsl)], ['Below LSL', share(stats.below)]);
        if (usl !== null) out.push(['USL', formatNumber(usl)], ['Above USL', share(stats.above)]);
        if (target !== null) out.push(['Target', formatNumber(target)], ['Off target', formatNumber(stats.mean - target), 'The average less the target']);
        out.push(
          ['Cp', index(fit.cp), 'The width between the limits over six short-term standard deviations: what the process could hold if centred. Needs both limits.'],
          ['Cpk', index(fit.cpk), 'The distance from the average to the nearer limit over three short-term standard deviations'],
          ['Pp', index(fit.pp), 'As Cp, with the standard deviation of all the values'],
          ['Ppk', index(fit.ppk), 'As Cpk, with the standard deviation of all the values: what the process actually did'],
        );
      }
    }
    if (is('pareto') && data.x.type === 'categories') {
      const values = (data.series[0]?.values ?? []).map((v) => (v !== null && v > 0 ? v : 0));
      const whole = values.reduce((a, b) => a + b, 0);
      if (whole > 0) {
        out.push(['Total', formatNumber(whole)], ['Values', formatCount(values.length)]);
        const cutoff = graph.paretoCutoff;
        if (cutoff > 0 && cutoff < 100) {
          // The bars it takes to reach the mark, the one that crosses it included.
          let [few, running] = [0, 0];
          while (few < values.length && (running / whole) * 100 < cutoff) running += values[few++];
          out.push([
            'Vital few',
            `${formatCount(few)} of ${formatCount(values.length)}`,
            `The first ${few} ${few === 1 ? 'value makes' : 'values make'} up ${((running / whole) * 100).toFixed(1)}% of the total`,
          ]);
          out.push(['Their share', `${((running / whole) * 100).toFixed(1)}%`]);
        }
        out.push(['Largest', `${data.x.labels[0]}  ${((values[0] / whole) * 100).toFixed(1)}%`]);
      }
    }
    return out;
  });

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
      label: 'Where the column goes',
      items: [
        { label: 'Put on the X axis', action: () => graph.put('x', column) },
        { label: 'Put on the Y axis', disabled: is('histogram'), action: () => graph.put('y', column) },
        { label: 'Colour by', disabled: !usesColor, action: () => graph.put('color', column) },
        separator,
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
    if (e.key !== 'Escape' || e.defaultPrevented || dialog) return;
    // In a field or a list, Esc belongs to that control (closing its list, undoing what was typed).
    const at = e.target;
    if (at instanceof HTMLInputElement || at instanceof HTMLSelectElement || at instanceof HTMLTextAreaElement) return;
    if (menu) menu = null;
    else onclose();
  }
</script>

<svelte:window {onkeydown} />

{#snippet zone(which: Zone, heading: string, off: string | null)}
  {@const disabled = off !== null}
  {@const chosen = which === 'color' ? graph.color : (graph[which]?.column ?? null)}
  {@const axis = which === 'color' ? null : graph[which]}
  <div class="zone" class:over={dragging?.over === which} class:filled={chosen !== null && !disabled} data-zone={disabled ? undefined : which}>
    <span class="zone-name">{heading}</span>
    <div class="zone-row">
      <select
        aria-label={`Column for ${heading}`}
        title={off ?? undefined}
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
      {@const cut = which === 'x' && is('bar', 'line', 'histogram', 'box', 'control')}
      <div class="zone-row">
        <!-- A pie and a Pareto chart are always of categories: there is nothing to choose. -->
        {#if roles.length > 1 && !(which === 'x' && is('pie', 'pareto'))}
          <label>
            <span class="muted">as</span>
            <select value={axis.role} onchange={(e) => graph.setRole(which, e.currentTarget.value as ChartRole)}>
              {#each roles as role}<option value={role}>{ROLE_LABELS[role]}</option>{/each}
            </select>
          </label>
        {/if}
        {#if cut && axis.role === 'time'}
          <label>
            <span class="muted">by</span>
            <select value={graph.bucket ?? ''} onchange={(e) => (graph.bucket = (e.currentTarget.value || null) as TimeBucket | null)}>
              <option value="">auto</option>
              {#each BUCKETS as unit}<option value={unit}>{unit}</option>{/each}
            </select>
          </label>
        {:else if cut && axis.role === 'continuous'}
          <label title="About how many stretches the numbers are cut into. Leave empty to have it chosen.">
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
        {:else if which === 'y' && is('bar', 'line', 'pie', 'pareto') && axis.role === 'continuous'}
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
        <button aria-pressed={graph.kind === k.kind} title={k.hint} onclick={() => graph.setKind(k.kind)}>{k.label}</button>
      {/each}
    </div>
    <span class="divider"></span>
    <button
      disabled={!graph.ready && graph.filters.length === 0 && graph.lines.length === 0 && !graph.trend && !graph.title && !graph.xLabel && !graph.yLabel}
      title="Start over: take everything off the axes and remove the filters, lines, trend line and names"
      onclick={() => graph.reset()}>Reset</button
    >
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
      {@render zone('x', 'X axis', null)}
      {@render zone('y', 'Y axis', is('histogram') ? 'A histogram counts the rows in each bin, so it has no use for a Y column' : null)}
      {@render zone('color', 'Colour by', usesColor ? null : 'This kind of chart is not split by colour')}
    </div>

    <div class="stage">
      {#if error}
        <div class="message"><p class="error-text">{error}</p></div>
      {:else if !graph.ready}
        <div class="message"><p class="muted">{graph.hint}</p></div>
      {:else if data && !empty}
        <ChartCanvas
          bind:this={canvas}
          {data}
          {view}
          {label}
          {zoom}
          onzoom={(x, y) => ((graph.xRange = x), y && (graph.yRange = y))}
          onreset={resetZoom}
          oncontext={chartMenu}
        />
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

    <div class="side right" aria-label="Filters, lines and options">
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

      {#if !is('pie')}
        <h3>Lines</h3>
        <div class="marks">
          {#each graph.lines as line (line.id)}
            {@const here = shown(line)}
            <div class="mark" class:unshown={!here} title={here ? undefined : 'Not on this chart: it was drawn at a value of something this chart does not show'}>
              <span class={`stroke ${line.color} ${line.style}`} aria-hidden="true"></span>
              <span class="mark-text">
                {here ? `${here.axis.toUpperCase()} = ` : ''}{lineValue(line)}{line.name ? `  ${line.name}` : ''}
                {#if line.role !== 'none' && line.name.trim().toLowerCase() !== LINE_ROLE_NAMES[line.role].toLowerCase()}<span class="muted">({LINE_ROLE_NAMES[line.role]})</span>{/if}
              </span>
              <button class="quiet" disabled={!here} title="Change this line" aria-label={`Change the line at ${lineValue(line)}`} onclick={() => here && lineDialog(here, true)}>&#9998;</button>
              <button class="quiet" title="Remove this line" aria-label={`Remove the line at ${lineValue(line)}`} onclick={() => graph.removeLine(line.id)}>&#215;</button>
            </div>
          {/each}
          <div class="mark-actions">
            <button title="Draw a line across the chart at a value of the Y axis: a limit, a target (or right-click the chart)" onclick={() => addLine('y', null)}>+ Line on Y</button>
            <button disabled={!scaleX} title={scaleX ? 'Draw an upright line at a value of the X axis (or right-click the chart)' : 'This chart has no scale of values along X'} onclick={() => addLine('x', null)}>+ Line on X</button>
          </div>
          {#if graph.trend || choices.any}
            <div class="mark">
              <span class="stroke ink dashed" aria-hidden="true"></span>
              <span class="mark-text">
                {#if graph.trend}
                  Trend: {TREND_LABELS[graph.trend.kind].toLowerCase()}
                  {#if !choices.any || (graph.trend.kind === 'moving' && !choices.moving)}<span class="muted">(not on this chart)</span>{/if}
                {:else}
                  <span class="muted">No trend line</span>
                {/if}
              </span>
              <button class="quiet" disabled={!choices.any} title="Set up the trend line (or right-click the chart)" aria-label="Set up the trend line" onclick={trendDialog}>&#9998;</button>
              {#if graph.trend}
                <button class="quiet" title="Remove the trend line" aria-label="Remove the trend line" onclick={() => (graph.trend = null)}>&#215;</button>
              {/if}
            </div>
            {#each trends as line}
              <p class="fit">{line.name === null ? '' : `${line.name}: `}{line.text}</p>
            {/each}
          {/if}
        </div>
      {/if}

      {#if figures.length > 0}
        <h3>Figures</h3>
        <dl class="figures">
          {#each figures as [name, value, hint] (name)}
            <dt title={hint}>{name}</dt>
            <dd>{value}</dd>
          {/each}
        </dl>
        {#if graph.measured && !figures.some(([name]) => name === 'Cpk')}
          <p class="muted tip">For Cp and Cpk, add a line on the {graph.measured.toUpperCase()} axis and mark it as a specification limit.</p>
        {/if}
      {/if}

      <h3>Options</h3>
      <div class="options">
        {#if is('bar', 'line', 'box') && graph.x?.role === 'discrete'}
          <label>
            <span>Order</span>
            <select bind:value={graph.sort}>
              <option value="value">{is('box') ? 'Most rows first' : 'Largest first'}</option>
              <option value="label">By name</option>
              <option value="seen">As in the result</option>
            </select>
          </label>
        {/if}
        {#if is('bar', 'line', 'box', 'pie', 'pareto') && graph.x?.role === 'discrete'}
          <label title="More values than this are merged into one called Other">
            <span>{is('pie') ? 'Slices shown' : is('box') ? 'Boxes shown' : is('line') ? 'Points shown' : 'Bars shown'}</span>
            <input
              type="number"
              min="1"
              max={is('pie') ? 8 : 500}
              value={is('pie') ? Math.min(8, graph.maxCategories) : graph.maxCategories}
              onchange={(e) => (graph.maxCategories = Math.min(500, Math.max(1, Math.round(Number(e.currentTarget.value) || 40))))}
            />
          </label>
        {/if}
        {#if is('bar', 'line', 'pie', 'pareto') && graph.x?.role === 'discrete'}
          <label class="check" title="Rows where the X column is NULL get no bar or slice of their own">
            <input type="checkbox" bind:checked={graph.skipNull} /> <span>Leave out rows with no value</span>
          </label>
        {/if}
        {#if is('pareto') || (is('bar') && graph.x?.role === 'discrete')}
          <label class="check"><input type="checkbox" bind:checked={graph.barLabels} /> <span>Values on the bars</span></label>
        {/if}
        {#if is('pareto')}
          <label class="check"><input type="checkbox" bind:checked={graph.paretoLine} /> <span>Running total line</span></label>
          <label title="Where the running total is marked. The bars it takes to reach it are the vital few. 0 for no mark.">
            <span>Mark at (%)</span>
            <input
              type="number"
              min="0"
              max="100"
              step="5"
              value={graph.paretoCutoff}
              onchange={(e) => (graph.paretoCutoff = Math.min(100, Math.max(0, Math.round(Number(e.currentTarget.value) || 0))))}
            />
          </label>
          <label class="check" title="Draw the bars past the mark paler, so the few that matter stand out">
            <input type="checkbox" bind:checked={graph.paretoFade} disabled={graph.paretoCutoff <= 0} /> <span>Fade the bars past the mark</span>
          </label>
        {/if}
        {#if is('control')}
          {#if !graph.x}
            <label title="How many consecutive rows are averaged into one point. Leave empty for one row a point, or as many as it takes to fit.">
              <span>Rows per point</span>
              <input
                type="number"
                min="1"
                max="100000"
                placeholder="auto"
                value={graph.runSize ?? ''}
                onchange={(e) => (graph.runSize = e.currentTarget.value === '' ? null : Math.max(1, Math.round(Number(e.currentTarget.value) || 1)))}
              />
            </label>
          {/if}
          <label title="A centre line known from before (a target, or last month's average), to judge this data against. Leave empty to use this data's own average.">
            <span>Centre</span>
            <input type="text" placeholder="from this data" value={graph.fixedCenter ?? ''} onchange={(e) => (graph.fixedCenter = typedNumber(e.currentTarget.value))} />
          </label>
          <label title="A sigma known from before, for the limits. Leave empty to work it out from this data.">
            <span>Sigma</span>
            <input
              type="text"
              placeholder="from this data"
              value={graph.fixedSigma ?? ''}
              onchange={(e) => {
                const value = typedNumber(e.currentTarget.value);
                graph.fixedSigma = value !== null && value > 0 ? value : null;
              }}
            />
          </label>
          <label class="check" title={`A second chart underneath: the range from one point to the next, or the standard deviation within each point. It shows whether the spread itself is steady.`}>
            <input type="checkbox" bind:checked={graph.spreadChart} /> <span>Chart of the spread underneath</span>
          </label>
          <label class="check" title="The lines one and two sigma from the centre, which rules 5 to 8 are judged by">
            <input type="checkbox" bind:checked={graph.zones} /> <span>1 and 2 sigma lines</span>
          </label>
          <div class="rules" role="group" aria-label="Rules that mark a point">
            <span class="rules-name">Mark points that break</span>
            {#each RUN_RULES as rule (rule.rule)}
              <label class="check" title={rule.text}>
                <input type="checkbox" checked={graph.rules.includes(rule.rule)} onchange={(e) => toggleRule(rule.rule, e.currentTarget.checked)} />
                <span>{rule.rule}. {rule.short}</span>
              </label>
            {/each}
            <span class="rules-sets">
              <button class="link" title="Rules 1, 2, 5 and 6" onclick={() => (graph.rules = [1, 2, 5, 6])}>Western Electric</button>
              <button class="link" title="All eight" onclick={() => (graph.rules = [1, 2, 3, 4, 5, 6, 7, 8])}>Nelson</button>
              <button class="link" title="Only points beyond a limit" onclick={() => (graph.rules = [1])}>Limits only</button>
            </span>
          </div>
        {/if}
        {#if is('bar', 'histogram') && graph.color !== null}
          <label class="check"><input type="checkbox" bind:checked={graph.stacked} disabled={graph.logY} /> <span>Stack the colours</span></label>
        {/if}
        {#if is('histogram') && graph.x?.role === 'continuous'}
          <label class="check" title="The bell curve of a normal distribution with the same average and standard deviation as the data">
            <input type="checkbox" bind:checked={graph.normalCurve} /> <span>Normal curve</span>
          </label>
        {/if}
        {#if is('scatter')}
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
        {#if logAllowed}
          <label class="check" title="Each step up the Y axis is ten times the last. Values of zero or less are not drawn.">
            <input type="checkbox" bind:checked={graph.logY} /> <span>Log scale on Y</span>
          </label>
        {/if}
        {#if graph.color !== null && usesColor}
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
        {#if !is('pie')}
          <label><span>X axis name</span><input type="text" bind:value={graph.xLabel} placeholder={graph.name(graph.x?.column) || 'Automatic'} /></label>
          <label><span>Y axis name</span><input type="text" bind:value={graph.yLabel} placeholder="Automatic" /></label>
        {/if}
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
      <span class="muted">{zoom ? 'Drag across the chart to zoom in. ' : ''}{is('pie') ? 'Right-click it to give it a title.' : 'Right-click it for lines, a trend line and names.'}</span>
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
  <Menu items={menu.items} x={menu.x} y={menu.y} label={menu.label} onclose={() => (menu = null)} />
{/if}

<!-- Keyed, so a popup opened straight after another is a new one; and a popup only ever closes itself. -->
{#key dialog}
  {#if dialog}
    {@const shown = dialog}
    <GraphDialog request={shown} onclose={() => dialog === shown && (dialog = null)} />
  {/if}
{/key}

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
    padding-left: 9px;
    padding-right: 9px;
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
    grid-template-columns: 232px minmax(0, 1fr) 264px;
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

  .marks {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 12px;
  }

  .mark {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 22px;
  }

  .mark.unshown .mark-text {
    color: var(--muted);
    text-decoration: line-through;
  }

  .mark-text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mark button {
    padding: 0 5px;
  }

  .mark-actions {
    display: flex;
    gap: 6px;
    margin: 2px 0;
  }

  .mark-actions button {
    flex: 1;
    padding: 2px 4px;
    font-size: 12px;
  }

  .stroke {
    width: 22px;
    flex: none;
    border-top: 2px solid var(--muted);
  }

  .stroke.dashed {
    border-top-style: dashed;
  }

  .stroke.dotted {
    border-top-style: dotted;
  }

  .stroke.red {
    border-top-color: var(--danger);
  }

  .stroke.green {
    border-top-color: var(--ok);
  }

  .stroke.blue {
    border-top-color: var(--accent);
  }

  .stroke.ink {
    border-top-color: var(--text);
  }

  .fit {
    margin: 0 0 0 28px;
    font-size: 12px;
    color: var(--muted);
    overflow-wrap: anywhere;
    font-variant-numeric: tabular-nums;
  }

  .figures {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 10px;
    margin: 0;
    font-size: 12px;
  }

  .figures dt {
    color: var(--muted);
  }

  .figures dt[title] {
    text-decoration: underline dotted;
    text-underline-offset: 2px;
    cursor: help;
  }

  .figures dd {
    margin: 0;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .tip {
    margin: 2px 0 0;
    font-size: 12px;
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
    width: 82px;
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

  .rules {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 2px 0;
    padding: 5px 7px 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }

  .rules-name {
    color: var(--muted);
    margin-bottom: 2px;
  }

  .rules-sets {
    display: flex;
    gap: 10px;
    margin-top: 3px;
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
