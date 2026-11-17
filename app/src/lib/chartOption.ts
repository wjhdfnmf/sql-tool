// Turns what the backend worked out into what the chart library draws.
//
// Everything a value from the database could reach is drawn on the canvas or
// set with textContent. Nothing here builds HTML from data: a tooltip is a
// DOM element made by hand, never a string of markup.

import {
  OTHER_LABEL,
  SEQUENTIAL,
  SERIES_DARK,
  SERIES_LIGHT,
  TREND_LABELS,
  describeFit,
  evalFit,
  fitPoints,
  fitQuality,
  formatBucket,
  formatNumber,
  formatTime,
  mixColors,
  movingAverage,
  normalDensity,
} from './chart';
import type { ChartData, ChartKind, Fit, RefLine, TimeBucket } from './chart';
import type { TrendSetup } from './graph.svelte';

/** Colours and type taken from the theme in use. */
export interface ChartLook {
  text: string;
  muted: string;
  border: string;
  bg: string;
  panel: string;
  accent: string;
  danger: string;
  ok: string;
  dark: boolean;
  font: string;
}

/** How the chart is drawn, beyond the data. */
export interface ChartView {
  kind: ChartKind;
  stacked: boolean;
  legend: 'right' | 'bottom' | 'hidden';
  title: string;
  /** Axis names, already worked out ("Sum of amount", "placed_at by month"). */
  xName: string;
  yName: string;
  logY: boolean;
  pointSize: number;
  /** The X and Y columns hold times. */
  xTime: boolean;
  yTime: boolean;
  /** Bounds to hold the axes to, when zoomed in. */
  xRange: [number, number] | null;
  yRange: [number, number] | null;
  /** The colour slot of a named group. */
  slot: (name: string) => number;
  lines: RefLine[];
  trend: TrendSetup | null;
  normalCurve: boolean;
}

type Options = Record<string, unknown>;
type TipRow = { swatch?: string; label: string; value: string };

const compact = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 2 });
const axisNumber = (v: number) => (Math.abs(v) >= 10_000 ? compact.format(v) : formatNumber(v));

/**
 * Labels for a number axis held to exact bounds (a zoomed-in plot). The
 * chart library labels the two ends as well as the round values between
 * them; an end such as 3,186.56 is noise, so it is left blank unless it
 * happens to be round for the stretch shown.
 */
function boundedNumber(bounds: [number, number] | null) {
  if (!bounds || !(bounds[1] > bounds[0])) return axisNumber;
  const unit = 10 ** Math.floor(Math.log10(bounds[1] - bounds[0])) / 20;
  const round = (v: number) => Math.abs(v / unit - Math.round(v / unit)) < 1e-6;
  return (v: number) => ((v === bounds[0] || v === bounds[1]) && !round(v) ? '' : axisNumber(v));
}

/** A tooltip's content, built as elements so that no value is ever read as markup. */
function tip(title: string, rows: TipRow[]): HTMLElement {
  const box = document.createElement('div');
  box.className = 'chart-tip';
  if (title) {
    const head = document.createElement('div');
    head.className = 'chart-tip-title';
    head.textContent = title;
    box.append(head);
  }
  for (const row of rows) {
    const line = document.createElement('div');
    line.className = 'chart-tip-row';
    if (row.swatch) {
      const swatch = document.createElement('span');
      swatch.className = 'chart-tip-swatch';
      swatch.style.background = row.swatch;
      line.append(swatch);
    }
    const label = document.createElement('span');
    label.textContent = row.label;
    const value = document.createElement('b');
    value.textContent = row.value;
    line.append(label, value);
    box.append(line);
  }
  return box;
}

function binLabel(from: number, to: number, time: boolean, bucket: TimeBucket | null, timeOnly: boolean): string {
  if (time) return bucket && bucket !== 'week' ? formatBucket(from, bucket, timeOnly) : `${formatTime(from, timeOnly)} to ${formatTime(to, timeOnly)}`;
  return `${formatNumber(from)} to ${formatNumber(to)}`;
}

/** The shade for a count on the pale-to-deep ramp. On a dark background the ramp runs the other way, so "few" still fades into it. */
function shade(count: number, max: number, dark: boolean): string {
  const t = max <= 1 ? 1 : Math.log1p(count) / Math.log1p(max);
  const at = (dark ? 1 - t : t) * (SEQUENTIAL.length - 1);
  const low = Math.min(SEQUENTIAL.length - 2, Math.floor(at));
  return mixColors(SEQUENTIAL[low], SEQUENTIAL[low + 1], at - low);
}

/** Whether lines can be drawn at values of X: only where X is a scale of numbers or time, not a row of names. */
export function xIsScale(data: ChartData, kind: ChartKind): boolean {
  if (data.x.type === 'continuous') return true;
  return data.x.type === 'bins' && (kind === 'bar' || kind === 'line' || kind === 'histogram');
}

/** Whether this chart can carry a trend line, and whether a moving average is among the choices. */
export function trendChoices(data: ChartData | null, kind: ChartKind): { any: boolean; moving: boolean; perGroup: boolean } {
  if (!data) return { any: false, moving: false, perGroup: false };
  if (kind === 'scatter') return { any: true, moving: false, perGroup: data.series.length > 1 };
  const ordered = kind === 'control' || ((kind === 'line' || kind === 'bar') && data.x.type === 'bins');
  return { any: ordered, moving: ordered, perGroup: ordered && data.series.length > 1 };
}

/** The names along X for charts that are drawn on a row of names. */
function axisLabels(data: ChartData): string[] {
  const x = data.x;
  if (x.type === 'categories') return x.labels;
  if (x.type === 'bins') return x.edges.slice(0, -1).map((from, i) => binLabel(from, x.edges[i + 1], x.time, x.bucket, data.xTimeOnly));
  if (x.type === 'sequence') return Array.from({ length: x.count }, (_, i) => String(i * x.rowsPerPoint + 1));
  return [];
}

/** A trend line worked out for the chart: where it runs, and what to say about it. */
export interface TrendLine {
  /** The group it belongs to, or null for one line through everything. */
  name: string | null;
  /** Its height at each place along X: [x, y] pairs for a scale, or one value per name. */
  points: (number | null)[][] | (number | null)[];
  /** The equation and fit, or what a moving average is of. */
  text: string;
}

/**
 * The trend lines of a chart. A scatter plot's come from the backend, fitted
 * to every row. The other charts have few points, so theirs are fitted here.
 */
export function trendLines(data: ChartData, view: ChartView): TrendLine[] {
  const setup = view.trend;
  const choices = trendChoices(data, view.kind);
  if (!setup || !choices.any) return [];
  const name = (index: number | null) => (index === null ? null : (data.series[index]?.name ?? null));

  if (view.kind === 'scatter') {
    if (setup.kind === 'moving') return [];
    let [low, high] = view.xRange ?? data.density?.x ?? [Infinity, -Infinity];
    if (!view.xRange && !data.density) {
      for (const s of data.series) {
        for (let i = 0; i < s.points.length; i += 2) {
          low = Math.min(low, s.points[i]);
          high = Math.max(high, s.points[i]);
        }
      }
    }
    if (!(high > low)) return [];
    return data.fits.map((fit) => {
      const points: number[][] = [];
      for (let i = 0; i <= 80; i++) {
        const x = low + ((high - low) * i) / 80;
        const y = evalFit(fit, x);
        if (Number.isFinite(y)) points.push([x, y]);
      }
      return { name: name(fit.series), points, text: describeFit(fit, view.xTime) };
    });
  }

  // Charts drawn point by point along X: each series is fitted on its own, or their total as one.
  const x = data.x;
  const onScale = x.type === 'bins' && view.kind !== 'control';
  const places = x.type === 'bins' && onScale ? x.edges.slice(0, -1).map((from, i) => (from + x.edges[i + 1]) / 2) : data.series[0]?.values.map((_, i) => i + 1) ?? [];
  const together = !setup.perGroup || data.series.length < 2;
  const lines = together
    ? [{ name: null as string | null, values: places.map((_, i) => total(data.series.map((s) => s.values[i]))) }]
    : data.series.map((s) => ({ name: s.name, values: s.values }));
  const out: TrendLine[] = [];
  for (const line of lines) {
    let heights: (number | null)[];
    let text: string;
    if (setup.kind === 'moving') {
      heights = movingAverage(line.values, setup.window);
      text = `Average of ${Math.max(2, Math.round(setup.window))} points`;
    } else {
      const used = line.values.map((v, i) => [places[i], v] as const).filter((pair): pair is readonly [number, number] => pair[1] !== null);
      const fit: Fit | null = fitPoints(used.map((p) => p[0]), used.map((p) => p[1]), setup.kind, setup.degree);
      if (!fit) continue;
      heights = places.map((place) => {
        const y = evalFit(fit, place);
        return Number.isFinite(y) ? y : null;
      });
      // Along a row of names the "x" is only the position, so the equation's numbers mean nothing: give the fit alone.
      text = onScale ? describeFit(fit, x.type === 'bins' && x.time) : `${TREND_LABELS[setup.kind]}, ${fitQuality(fit)}`;
    }
    out.push({ name: line.name, points: onScale ? heights.map((h, i) => [places[i], h]) : heights, text });
  }
  return out;
}

/** The sum of the values that are there; null if none is. */
function total(values: (number | null)[]): number | null {
  const present = values.filter((v): v is number => v !== null);
  return present.length ? present.reduce((a, b) => a + b, 0) : null;
}

export function buildOption(data: ChartData, view: ChartView, look: ChartLook): Options {
  const palette = look.dark ? SERIES_DARK : SERIES_LIGHT;
  const colorOf = (name: string | null) => (name === null ? palette[0] : name === OTHER_LABEL ? look.muted : palette[view.slot(name) % palette.length]);
  const names = data.series.map((s) => s.name).filter((name): name is string => name !== null);
  const legendShown = names.length > 0 && view.legend !== 'hidden' && !data.density && view.kind !== 'pie';
  const legendRight = legendShown && view.legend === 'right';
  const gridRight = legendRight ? 150 : view.kind === 'pareto' ? 12 : 20;
  const gridTop = view.title ? 38 : 14;

  const axis = {
    axisLine: { lineStyle: { color: look.border } },
    axisTick: { lineStyle: { color: look.border } },
    axisLabel: { color: look.muted, fontSize: 11, hideOverlap: true },
    nameTextStyle: { color: look.muted, fontSize: 12 },
    nameLocation: 'middle',
    splitLine: { lineStyle: { color: look.border, opacity: 0.55 } },
  };
  const numberLabel = { ...axis.axisLabel, formatter: axisNumber };
  const timeLabel = (timeOnly: boolean) => (timeOnly ? { ...axis.axisLabel, formatter: (v: number) => formatTime(v, true) } : axis.axisLabel);
  const yValue = (v: number) => (view.yTime ? formatTime(v, data.yTimeOnly) : formatNumber(v));

  const titles: Options[] = [];
  if (view.title) titles.push({ text: view.title, left: 'center', top: 4, textStyle: { color: look.text, fontSize: 14, fontWeight: 600 } });

  const option: Options = {
    useUTC: true,
    animation: false,
    backgroundColor: 'transparent',
    textStyle: { color: look.text, fontFamily: look.font },
    title: titles,
    legend: legendShown
      ? {
          type: 'scroll',
          data: names,
          itemWidth: 12,
          itemHeight: 12,
          textStyle: { color: look.text, fontSize: 12, width: 110, overflow: 'truncate' },
          inactiveColor: look.border,
          pageTextStyle: { color: look.muted },
          pageIconColor: look.muted,
          pageIconInactiveColor: look.border,
          ...(legendRight ? { orient: 'vertical', right: 6, top: 'middle' } : { orient: 'horizontal', bottom: 4, left: 'center' }),
        }
      : undefined,
    grid: {
      left: 12,
      right: gridRight,
      top: gridTop,
      bottom: legendShown && !legendRight ? 38 : 10,
      outerBoundsMode: 'same',
      outerBoundsContain: 'all',
    },
    tooltip: {
      confine: true,
      backgroundColor: look.panel,
      borderColor: look.border,
      padding: [6, 9],
      textStyle: { color: look.text, fontSize: 12 },
      transitionDuration: 0,
    },
  };
  const tooltip = option.tooltip as Options;

  // ---- a pie has no axes: it is done here ----
  if (view.kind === 'pie' && data.x.type === 'categories') {
    const labels = data.x.labels;
    const slices = labels
      .map((label, i) => ({ name: label, value: data.series[0]?.values[i] ?? 0, itemStyle: { color: label === OTHER_LABEL ? look.muted : palette[i % palette.length] } }))
      .filter((slice) => slice.value > 0);
    const whole = slices.reduce((sum, slice) => sum + slice.value, 0);
    const share = (value: number) => (whole > 0 ? `${((value / whole) * 100).toFixed(value / whole < 0.1 ? 1 : 0)}%` : '');
    option.series = [
      {
        type: 'pie',
        name: view.yName,
        radius: '68%',
        center: ['50%', view.title ? '54%' : '50%'],
        data: slices,
        // Largest first from twelve o'clock, clockwise: the order the backend sends them in.
        startAngle: 90,
        clockwise: true,
        avoidLabelOverlap: true,
        // A ring of background between slices, so two fills never touch.
        itemStyle: { borderColor: look.bg, borderWidth: 2 },
        label: { color: look.text, fontSize: 12, formatter: (item: { name: string; value: number }) => `${item.name}  ${share(item.value)}` },
        labelLine: { lineStyle: { color: look.border } },
        emphasis: { scaleSize: 4 },
      },
    ];
    tooltip.trigger = 'item';
    tooltip.formatter = (item: { name: string; value: number; color: string }) =>
      tip(item.name, [
        { swatch: item.color, label: view.yName, value: formatNumber(item.value) },
        { label: 'Share', value: share(item.value) },
      ]);
    return option;
  }

  // ---- lines drawn across at chosen values, and room on the axes for them ----
  const scaleX = xIsScale(data, view.kind);
  const drawn = view.lines.filter((line) => line.axis === 'y' || scaleX);
  const lineColors = { red: look.danger, green: look.ok, blue: look.accent, grey: look.muted };
  const reach = (which: 'x' | 'y', extra: number[] = []) => {
    const values = [...drawn.filter((line) => line.axis === which).map((line) => line.value), ...extra];
    return values.length ? ([Math.min(...values), Math.max(...values)] as const) : null;
  };
  /** Widens an axis the library would fit to the data, so that it also takes in the lines. Null leaves it to the library. */
  const widened = (span: readonly [number, number] | null) =>
    span
      ? {
          min: (found: { min: number }) => (span[0] < found.min ? span[0] : null),
          max: (found: { max: number }) => (span[1] > found.max ? span[1] : null),
        }
      : {};
  const markers = (extra: Options[] = []): Options => ({
    symbol: 'none',
    silent: true,
    animation: false,
    data: [
      ...drawn.map((line) => {
        const value = line.axis === 'x' && view.xTime ? formatTime(line.value, data.xTimeOnly) : line.axis === 'y' ? yValue(line.value) : formatNumber(line.value);
        const text = line.name.trim() ? `${line.name.trim()}  ${value}` : value;
        return {
          [line.axis === 'x' ? 'xAxis' : 'yAxis']: line.value,
          lineStyle: { color: lineColors[line.color], type: line.style, width: 1.5 },
          label: { show: true, formatter: () => text, position: 'insideEndTop', color: look.text, fontSize: 11 },
        };
      }),
      ...extra,
    ],
  });
  /** An empty series that only carries the lines, so they do not depend on which series the chart happens to have. */
  const lineCarrier = (extra: Options[] = []): Options => ({ type: 'line', name: '', data: [], silent: true, markLine: markers(extra), tooltip: { show: false } });

  const yAxis: Options = {
    ...axis,
    type: view.yTime ? 'time' : view.logY ? 'log' : 'value',
    name: view.yName,
    axisLabel: view.yTime ? timeLabel(data.yTimeOnly) : numberLabel,
    ...widened(reach('y')),
  };

  // ---- trend lines, on whichever axes the chart has ----
  const trends = trendLines(data, view);
  const trendSeries = (): Options[] =>
    trends.map((line) => ({
      type: 'line',
      name: line.name === null ? 'Trend' : `Trend: ${line.name}`,
      data: line.points.map((point) => (point === null ? '-' : point)),
      showSymbol: false,
      connectNulls: false,
      // One line through everything is drawn in ink; a line per group in the group's colour.
      lineStyle: { color: line.name === null ? look.text : colorOf(line.name), width: 2, type: view.trend?.kind === 'moving' ? 'solid' : 'dashed' },
      itemStyle: { color: line.name === null ? look.text : colorOf(line.name) },
      tooltip: { show: false },
      silent: true,
      z: 5,
    }));
  if (view.trend?.showEquation && trends.length > 0) {
    titles.push({
      text: trends.map((line) => (line.name === null ? line.text : `${line.name}: ${line.text}`)).join('\n'),
      left: 84,
      top: gridTop + 6,
      textStyle: { color: look.muted, fontSize: 11, fontWeight: 400, lineHeight: 15 },
    });
  }

  // ---- charts drawn on a row of names ----
  const named = data.x.type === 'categories' || data.x.type === 'sequence' || (data.x.type === 'bins' && (view.kind === 'box' || view.kind === 'control'));
  if (named) {
    const labels = axisLabels(data);
    const nameAxis: Options = {
      ...axis,
      type: 'category',
      data: labels,
      name: view.xName,
      splitLine: { show: false },
      axisTick: { ...axis.axisTick, alignWithLabel: true },
      axisLabel: { ...axis.axisLabel, rotate: labels.length > 8 && data.x.type !== 'sequence' ? 35 : 0, width: 120, overflow: 'truncate' },
    };
    option.xAxis = nameAxis;
    option.yAxis = yAxis;
    tooltip.trigger = 'axis';
    tooltip.axisPointer = { type: 'shadow', shadowStyle: { color: look.border, opacity: 0.25 } };

    if (view.kind === 'box') {
      const fill = mixColors(look.bg, palette[0], 0.22);
      option.yAxis = { ...yAxis, type: view.yTime ? 'time' : 'value', scale: true };
      option.series = [
        {
          type: 'boxplot',
          name: view.yName,
          data: data.boxes.map((box) => (box ? [box.low, box.q1, box.median, box.q3, box.high] : ['-', '-', '-', '-', '-'])),
          itemStyle: { color: fill, borderColor: palette[0], borderWidth: 1.5 },
          boxWidth: [8, 56],
          emphasis: { disabled: true },
        },
        {
          type: 'scatter',
          name: 'Outliers',
          data: data.boxes.flatMap((box, i) => (box ? box.outliers.map((value) => [i, value]) : [])),
          symbolSize: 5,
          itemStyle: { color: palette[0], opacity: 0.55 },
          tooltip: { show: false },
          silent: true,
        },
        lineCarrier(),
      ];
      tooltip.trigger = 'item';
      tooltip.formatter = (item: { dataIndex: number }) => {
        const box = data.boxes[item.dataIndex];
        if (!box) return tip(labels[item.dataIndex] ?? '', []);
        const rows: TipRow[] = [
          { label: 'Rows', value: formatNumber(box.rows) },
          { label: 'Median', value: yValue(box.median) },
          { label: 'Middle half', value: `${yValue(box.q1)} to ${yValue(box.q3)}` },
          { label: 'Whiskers', value: `${yValue(box.low)} to ${yValue(box.high)}` },
          { label: 'Smallest, largest', value: `${yValue(box.min)}, ${yValue(box.max)}` },
        ];
        if (!view.yTime) rows.splice(2, 0, { label: 'Average', value: formatNumber(box.mean) });
        if (box.outlierCount > 0) rows.push({ label: 'Outliers', value: formatNumber(box.outlierCount) });
        return tip(labels[item.dataIndex] ?? '', rows);
      };
      return option;
    }

    if (view.kind === 'control' && data.control) {
      const control = data.control;
      const means = data.series[0]?.values ?? [];
      const outside = (i: number) => {
        const [m, u, l] = [means[i], control.upper[i], control.lower[i]];
        return m !== null && u !== null && l !== null && (m > u || m < l);
      };
      const uppers = control.upper.filter((v): v is number => v !== null);
      const lowers = control.lower.filter((v): v is number => v !== null);
      // With every point the same size the limits are two straight lines; otherwise they step with the size.
      const level = uppers.length > 0 && Math.max(...uppers) - Math.min(...uppers) < 1e-9 * Math.max(1, Math.abs(uppers[0]));
      const limitStyle = { color: look.danger, type: 'dashed', width: 1.2 };
      const edge = (value: number, text: string, style: Options): Options => ({
        yAxis: value,
        lineStyle: style,
        label: { show: true, formatter: () => `${text}  ${formatNumber(value)}`, position: 'insideEndTop', color: look.text, fontSize: 11 },
      });
      const fixed: Options[] = [edge(control.center, 'Centre', { color: look.ok, type: 'solid', width: 1.2 })];
      if (level) fixed.push(edge(uppers[0], 'UCL', limitStyle), edge(lowers[0], 'LCL', limitStyle));
      const step = (values: (number | null)[], name: string): Options => ({
        type: 'line',
        name,
        data: values.map((v) => (v === null ? '-' : v)),
        step: 'middle',
        showSymbol: false,
        // Named at its right end, as the level limits are.
        endLabel: { show: true, formatter: () => name, color: look.text, fontSize: 11, offset: [-28, name === 'UCL' ? -8 : 10] },
        lineStyle: limitStyle,
        itemStyle: { color: look.danger },
        silent: true,
        tooltip: { show: false },
      });
      option.yAxis = { ...yAxis, type: 'value', scale: true, ...widened(reach('y', level ? [uppers[0], lowers[0], control.center] : [control.center])) };
      option.series = [
        {
          type: 'line',
          name: view.yName,
          data: means.map((m) => (m === null ? '-' : m)),
          symbol: 'circle',
          symbolSize: means.length > 150 ? 3 : 6,
          showSymbol: means.length <= 600,
          showAllSymbol: means.length <= 150,
          connectNulls: true,
          lineStyle: { color: palette[0], width: 1.5 },
          itemStyle: { color: palette[0] },
          emphasis: { disabled: true },
        },
        ...(level ? [] : [step(control.upper, 'UCL'), step(control.lower, 'LCL')]),
        {
          type: 'scatter',
          name: 'Outside the limits',
          data: means.flatMap((m, i) => (m !== null && outside(i) ? [[i, m]] : [])),
          symbolSize: 9,
          itemStyle: { color: look.danger, borderColor: look.bg, borderWidth: 1 },
          silent: true,
          tooltip: { show: false },
          z: 4,
        },
        ...trendSeries(),
        lineCarrier(fixed),
      ];
      tooltip.axisPointer = { type: 'line', lineStyle: { color: look.border } };
      tooltip.formatter = (items: { dataIndex: number }[]) => {
        const at = items[0]?.dataIndex ?? 0;
        const [m, u, l] = [means[at], control.upper[at], control.lower[at]];
        if (m === null || u === null || l === null) return tip(labels[at] ?? '', []);
        const rows: TipRow[] = [
          { swatch: outside(at) ? look.danger : palette[0], label: view.yName, value: formatNumber(m) },
          { label: 'Limits', value: `${formatNumber(l)} to ${formatNumber(u)}` },
        ];
        if (outside(at)) rows.push({ label: 'Outside the limits', value: m > u ? 'above' : 'below' });
        const x = data.x;
        const title = x.type === 'sequence' ? (x.rowsPerPoint > 1 ? `Rows ${labels[at]} to ${formatNumber(at * x.rowsPerPoint + x.rowsPerPoint)}` : `Row ${labels[at]}`) : (labels[at] ?? '');
        return tip(title, rows);
      };
      return option;
    }

    if (view.kind === 'pareto') {
      const values = (data.series[0]?.values ?? []).map((v) => (v !== null && v > 0 ? v : 0));
      const whole = values.reduce((a, b) => a + b, 0);
      let running = 0;
      const share = values.map((v) => (whole > 0 ? ((running += v) / whole) * 100 : 0));
      // One scale read two ways: the whole on the left is 100% on the right, so the line is the running total on both.
      option.yAxis = [
        { ...yAxis, type: 'value', min: 0, max: whole > 0 ? whole : undefined, ...(reach('y') && whole > 0 ? { max: Math.max(whole, reach('y')![1]) } : {}) },
        {
          ...axis,
          type: 'value',
          min: 0,
          max: reach('y') && whole > 0 ? (Math.max(whole, reach('y')![1]) / whole) * 100 : 100,
          name: 'Running total',
          splitLine: { show: false },
          axisLabel: { ...axis.axisLabel, formatter: (v: number) => (v <= 100 ? `${Math.round(v)}%` : '') },
        },
      ];
      (option.grid as Options).right = gridRight;
      option.series = [
        {
          type: 'bar',
          name: view.yName,
          data: values,
          barMaxWidth: 64,
          itemStyle: { color: palette[0], borderColor: look.bg, borderWidth: 1, borderRadius: [3, 3, 0, 0] },
          emphasis: { disabled: true },
        },
        {
          type: 'line',
          name: 'Running total',
          yAxisIndex: 1,
          data: share,
          symbol: 'circle',
          symbolSize: 6,
          lineStyle: { color: look.text, width: 2 },
          itemStyle: { color: look.text },
          emphasis: { disabled: true },
          // The usual mark: where the running total passes four fifths.
          markLine: {
            symbol: 'none',
            silent: true,
            data: [{ yAxis: 80, lineStyle: { color: look.muted, type: 'dotted', width: 1 }, label: { show: true, formatter: () => '80%', position: 'insideEndTop', color: look.muted, fontSize: 11 } }],
          },
        },
        lineCarrier(),
      ];
      tooltip.formatter = (items: { dataIndex: number }[]) => {
        const at = items[0]?.dataIndex ?? 0;
        return tip(labels[at] ?? '', [
          { swatch: palette[0], label: view.yName, value: formatNumber(values[at]) },
          { label: 'Share', value: whole > 0 ? `${((values[at] / whole) * 100).toFixed(1)}%` : '' },
          { label: 'Running total', value: `${share[at].toFixed(1)}%` },
        ]);
      };
      return option;
    }

    // Bars or lines over categories.
    const asLine = view.kind === 'line';
    const stacked = !asLine && view.stacked && !view.logY && data.series.length > 1;
    option.series = [
      ...data.series.map((s) => {
        const points = s.values.map((v) => (v === null || (view.logY && v <= 0) ? '-' : v));
        return asLine
          ? {
              type: 'line',
              name: s.name ?? view.yName,
              data: points,
              symbol: 'circle',
              symbolSize: labels.length > 60 ? 3 : 7,
              lineStyle: { color: colorOf(s.name), width: 2 },
              itemStyle: { color: colorOf(s.name), borderColor: look.bg, borderWidth: labels.length > 60 ? 0 : 1 },
              emphasis: { disabled: true },
            }
          : {
              type: 'bar',
              name: s.name ?? view.yName,
              data: points,
              stack: stacked ? 'all' : undefined,
              barMaxWidth: 56,
              barGap: '8%',
              // A hairline of background between neighbours, so two fills never touch.
              itemStyle: { color: colorOf(s.name), borderColor: look.bg, borderWidth: 1, borderRadius: stacked ? 0 : [3, 3, 0, 0] },
              emphasis: { disabled: true },
            };
      }),
      lineCarrier(),
    ];
    if (asLine) tooltip.axisPointer = { type: 'line', lineStyle: { color: look.border } };
    tooltip.formatter = (items: { dataIndex: number }[]) => {
      const at = items[0]?.dataIndex ?? 0;
      return tip(
        labels[at] ?? '',
        data.series.filter((s) => s.values[at] !== null).map((s) => ({ swatch: colorOf(s.name), label: s.name ?? view.yName, value: formatNumber(s.values[at] as number) })),
      );
    };
    return option;
  }

  if (data.x.type === 'bins') {
    const { edges, time, bucket } = data.x;
    const bins = Math.max(0, edges.length - 1);
    const count = data.series.length;
    const asLine = view.kind === 'line';
    const stacked = !asLine && view.stacked && !view.logY && count > 1;
    const span = reach('x', bins > 0 ? [edges[0], edges[bins]] : []);
    option.xAxis = {
      ...axis,
      type: time ? 'time' : 'value',
      min: span?.[0],
      max: span?.[1],
      name: view.xName,
      splitLine: { show: false },
      axisLabel: time ? timeLabel(data.xTimeOnly) : numberLabel,
    };
    option.yAxis = yAxis;
    const binTip = (bin: number) =>
      tip(
        binLabel(edges[bin], edges[bin + 1], time, bucket, data.xTimeOnly),
        data.series
          .filter((s) => s.values[bin] !== null && (asLine || s.values[bin] !== 0))
          .map((s) => ({ swatch: colorOf(s.name), label: s.name ?? view.yName, value: formatNumber(s.values[bin] as number) })),
      );
    tooltip.trigger = 'item';

    if (asLine) {
      option.series = [
        ...data.series.map((s) => ({
          type: 'line',
          name: s.name ?? view.yName,
          // Each point sits in the middle of its stretch of X. Its place in the list is kept for the tooltip.
          data: s.values.map((v, bin) => (v === null || (view.logY && v <= 0) ? null : [(edges[bin] + edges[bin + 1]) / 2, v, bin])).filter((p) => p !== null),
          symbol: 'circle',
          symbolSize: bins > 60 ? 3 : 7,
          lineStyle: { color: colorOf(s.name), width: 2 },
          itemStyle: { color: colorOf(s.name), borderColor: look.bg, borderWidth: bins > 60 ? 0 : 1 },
          emphasis: { scale: 1.4 },
        })),
        ...trendSeries(),
        lineCarrier(),
      ];
      tooltip.formatter = (item: { data: number[] }) => binTip(item.data[2]);
      return option;
    }

    // Where each bar of each series sits: [from, to, bottom, top, value, bin].
    const up = new Array<number>(bins).fill(0);
    const down = new Array<number>(bins).fill(0);
    const items = data.series.map((s, index) => {
      const out: number[][] = [];
      for (let bin = 0; bin < bins; bin++) {
        const v = s.values[bin];
        if (v === null || v === 0 || (view.logY && v < 0)) continue;
        let [from, to, bottom, top] = [edges[bin], edges[bin + 1], 0, v];
        if (stacked) {
          const base = v >= 0 ? up : down;
          [bottom, top] = [base[bin], base[bin] + v];
          base[bin] = top;
        } else if (count > 1) {
          const width = (to - from) / count;
          [from, to] = [from + index * width, from + (index + 1) * width];
        }
        out.push([from, to, bottom, top, v, bin]);
      }
      return out;
    });
    const curve: Options[] = [];
    const stats = data.stats;
    if (view.kind === 'histogram' && view.normalCurve && stats && stats.stdDev > 0 && !time && bins > 0 && !view.logY) {
      const width = edges[1] - edges[0];
      const [from, to] = [Math.min(edges[0], stats.mean - 4 * stats.stdDev), Math.max(edges[bins], stats.mean + 4 * stats.stdDev)];
      const [low, high] = [Math.max(from, span?.[0] ?? from), Math.min(to, span?.[1] ?? to)];
      curve.push({
        type: 'line',
        name: 'Normal curve',
        // As tall as a histogram of perfectly normal data with this many rows and these bins would be.
        data: Array.from({ length: 121 }, (_, i) => {
          const x = low + ((high - low) * i) / 120;
          return [x, stats.rows * width * normalDensity(x, stats.mean, stats.stdDev)];
        }),
        showSymbol: false,
        smooth: true,
        lineStyle: { color: look.text, width: 1.5 },
        itemStyle: { color: look.text },
        tooltip: { show: false },
        silent: true,
        z: 5,
      });
    }
    option.series = [
      ...data.series.map((s, index) => ({
        type: 'custom',
        name: s.name ?? view.yName,
        data: items[index],
        // A log axis has no zero for a bar to stand on: it scales to the tops alone.
        encode: { x: [0, 1], y: view.logY ? 3 : [2, 3] },
        itemStyle: { color: colorOf(s.name) },
        clip: true,
        renderItem: (params: { coordSys: { x: number; y: number; width: number; height: number } }, api: { value: (dim: number) => number; coord: (p: number[]) => number[] }) => {
          const [left, top] = api.coord([api.value(0), api.value(3)]);
          const right = api.coord([api.value(1), api.value(3)])[0];
          const floor = view.logY ? params.coordSys.y + params.coordSys.height : api.coord([api.value(0), api.value(2)])[1];
          // A pixel of background on each side, unless the bars are too thin to spare it.
          const gap = right - left > 4 ? 1 : 0;
          return {
            type: 'rect',
            shape: { x: left + gap, y: Math.min(top, floor), width: Math.max(1, right - left - 2 * gap), height: Math.max(1, Math.abs(floor - top)) },
            style: { fill: colorOf(s.name) },
          };
        },
      })),
      ...curve,
      ...trendSeries(),
      lineCarrier(),
    ];
    tooltip.formatter = (item: { data: number[] }) => binTip(item.data[5]);
    return option;
  }

  // ---- scatter: points, or a grid of counts when there are too many ----
  const xTime = data.x.type === 'continuous' && data.x.time;
  const value = (v: number, time: boolean, timeOnly: boolean) => (time ? formatTime(v, timeOnly) : formatNumber(v));
  /** The axis held to a zoom or to the shaded area, widened to take in any lines; otherwise left to the library, widened likewise. */
  const held = (range: [number, number] | null | undefined, which: 'x' | 'y') => {
    const lines = reach(which);
    if (!range) return widened(lines);
    return { min: Math.min(range[0], lines?.[0] ?? range[0]), max: Math.max(range[1], lines?.[1] ?? range[1]) };
  };
  option.xAxis = {
    ...axis,
    type: xTime ? 'time' : 'value',
    scale: true,
    name: view.xName,
    axisLabel: xTime ? timeLabel(data.xTimeOnly) : { ...axis.axisLabel, formatter: boundedNumber(view.xRange ?? data.density?.x ?? null) },
    ...held(view.xRange ?? data.density?.x, 'x'),
  };
  option.yAxis = { ...yAxis, scale: true, ...held(view.yRange ?? data.density?.y, 'y') };
  if (!view.yTime) (option.yAxis as Options).axisLabel = { ...axis.axisLabel, formatter: boundedNumber(view.yRange ?? data.density?.y ?? null) };
  tooltip.trigger = 'item';

  if (data.density) {
    const { cols, rows, x, y, counts, max } = data.density;
    const [width, height] = [(x[1] - x[0] || 1) / cols, (y[1] - y[0] || 1) / rows];
    const cells: number[][] = [];
    for (let i = 0; i < counts.length; i++) {
      if (counts[i] === 0) continue;
      const [col, line] = [i % cols, Math.floor(i / cols)];
      cells.push([x[0] + col * width, x[0] + (col + 1) * width, y[0] + line * height, y[0] + (line + 1) * height, counts[i]]);
    }
    option.series = [
      {
        type: 'custom',
        name: 'Rows',
        data: cells,
        encode: { x: [0, 1], y: [2, 3] },
        clip: true,
        renderItem: (_params: unknown, api: { value: (dim: number) => number; coord: (p: number[]) => number[] }) => {
          const [left, top] = api.coord([api.value(0), api.value(3)]);
          const [right, bottom] = api.coord([api.value(1), api.value(2)]);
          // Cells overlap by half a pixel so no seams show between them.
          return { type: 'rect', shape: { x: left, y: top, width: right - left + 0.5, height: bottom - top + 0.5 }, style: { fill: shade(api.value(4), max, look.dark) } };
        },
      },
      ...trendSeries(),
      lineCarrier(),
    ];
    tooltip.formatter = (item: { data: number[] }) => {
      const [x0, x1, y0, y1, n] = item.data;
      return tip(`${formatNumber(n)} ${n === 1 ? 'row' : 'rows'}`, [
        { label: view.xName, value: `${value(x0, xTime, data.xTimeOnly)} to ${value(x1, xTime, data.xTimeOnly)}` },
        { label: view.yName, value: `${value(y0, view.yTime, data.yTimeOnly)} to ${value(y1, view.yTime, data.yTimeOnly)}` },
      ]);
    };
    return option;
  }

  const points = data.series.reduce((n, s) => n + s.points.length / 2, 0);
  // Few points are drawn large enough to hit; many are drawn small and see-through so crowds show.
  const size = view.pointSize > 0 ? view.pointSize : points <= 300 ? 8 : points <= 5_000 ? 6 : 4;
  option.series = [
    ...data.series.map((s) => {
      const pairs: number[][] = [];
      for (let i = 0; i < s.points.length; i += 2) pairs.push([s.points[i], s.points[i + 1]]);
      return {
        type: 'scatter',
        name: s.name ?? view.yName,
        data: pairs,
        symbolSize: size,
        itemStyle: { color: colorOf(s.name), opacity: points > 5_000 ? 0.55 : 0.85, borderColor: look.bg, borderWidth: points <= 300 ? 1 : 0 },
        emphasis: { scale: 1.6 },
        large: points > 20_000,
        largeThreshold: 20_000,
      };
    }),
    ...trendSeries(),
    lineCarrier(),
  ];
  tooltip.formatter = (item: { data: number[]; seriesIndex: number }) =>
    tip(data.series[item.seriesIndex]?.name ?? '', [
      { label: view.xName, value: value(item.data[0], xTime, data.xTimeOnly) },
      { label: view.yName, value: value(item.data[1], view.yTime, data.yTimeOnly) },
    ]);
  return option;
}
