// Turns what the backend worked out into what the chart library draws.
//
// Everything a value from the database could reach is drawn on the canvas or
// set with textContent. Nothing here builds HTML from data: a tooltip is a
// DOM element made by hand, never a string of markup.

import { OTHER_LABEL, SEQUENTIAL, SERIES_DARK, SERIES_LIGHT, formatBucket, formatNumber, formatTime, mixColors } from './chart';
import type { ChartData, ChartKind, TimeBucket } from './chart';

/** Colours and type taken from the theme in use. */
export interface ChartLook {
  text: string;
  muted: string;
  border: string;
  bg: string;
  panel: string;
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
  /** The Y axis holds times. */
  yTime: boolean;
  /** Bounds to hold the axes to, when zoomed in. */
  xRange: [number, number] | null;
  yRange: [number, number] | null;
  /** The colour slot of a named group. */
  slot: (name: string) => number;
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

export function buildOption(data: ChartData, view: ChartView, look: ChartLook): Options {
  const palette = look.dark ? SERIES_DARK : SERIES_LIGHT;
  const colorOf = (name: string | null) => (name === null ? palette[0] : name === OTHER_LABEL ? look.muted : palette[view.slot(name) % palette.length]);
  const names = data.series.map((s) => s.name).filter((name): name is string => name !== null);
  const legendShown = names.length > 0 && view.legend !== 'hidden' && !data.density;
  const legendRight = legendShown && view.legend === 'right';

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

  const option: Options = {
    useUTC: true,
    animation: false,
    backgroundColor: 'transparent',
    textStyle: { color: look.text, fontFamily: look.font },
    title: view.title ? { text: view.title, left: 'center', top: 4, textStyle: { color: look.text, fontSize: 14, fontWeight: 600 } } : undefined,
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
      right: legendRight ? 150 : 20,
      top: view.title ? 38 : 14,
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
  const yAxis = {
    ...axis,
    type: view.yTime ? 'time' : view.logY ? 'log' : 'value',
    name: view.yName,
    axisLabel: view.yTime ? timeLabel(data.yTimeOnly) : numberLabel,
  };

  if (data.x.type === 'categories') {
    const labels = data.x.labels;
    const stacked = view.stacked && !view.logY && data.series.length > 1;
    option.xAxis = {
      ...axis,
      type: 'category',
      data: labels,
      name: view.xName,
      splitLine: { show: false },
      axisTick: { ...axis.axisTick, alignWithLabel: true },
      axisLabel: { ...axis.axisLabel, rotate: labels.length > 8 ? 35 : 0, width: 120, overflow: 'truncate' },
    };
    option.yAxis = yAxis;
    option.series = data.series.map((s) => ({
      type: 'bar',
      name: s.name ?? view.yName,
      data: s.values.map((v) => (v === null || (view.logY && v <= 0) ? '-' : v)),
      stack: stacked ? 'all' : undefined,
      barMaxWidth: 56,
      barGap: '8%',
      // A hairline of background between neighbours, so two fills never touch.
      itemStyle: { color: colorOf(s.name), borderColor: look.bg, borderWidth: 1, borderRadius: stacked ? 0 : [3, 3, 0, 0] },
      emphasis: { disabled: true },
    }));
    (option.tooltip as Options).trigger = 'axis';
    (option.tooltip as Options).axisPointer = { type: 'shadow', shadowStyle: { color: look.border, opacity: 0.25 } };
    (option.tooltip as Options).formatter = (items: { dataIndex: number; seriesIndex: number }[]) => {
      const at = items[0]?.dataIndex ?? 0;
      return tip(
        labels[at] ?? '',
        data.series
          .filter((s) => s.values[at] !== null)
          .map((s) => ({ swatch: colorOf(s.name), label: s.name ?? view.yName, value: formatNumber(s.values[at] as number) })),
      );
    };
    return option;
  }

  if (data.x.type === 'bins') {
    const { edges, time, bucket } = data.x;
    const bins = Math.max(0, edges.length - 1);
    const count = data.series.length;
    const stacked = view.stacked && !view.logY && count > 1;
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
    option.xAxis = {
      ...axis,
      type: time ? 'time' : 'value',
      min: edges[0],
      max: edges[bins],
      name: view.xName,
      splitLine: { show: false },
      axisLabel: time ? timeLabel(data.xTimeOnly) : numberLabel,
    };
    option.yAxis = yAxis;
    option.series = data.series.map((s, index) => ({
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
    }));
    (option.tooltip as Options).trigger = 'item';
    (option.tooltip as Options).formatter = (item: { data: number[]; seriesIndex: number }) => {
      const bin = item.data[5];
      return tip(
        binLabel(edges[bin], edges[bin + 1], time, bucket, data.xTimeOnly),
        data.series
          .filter((s) => s.values[bin] !== null && s.values[bin] !== 0)
          .map((s) => ({ swatch: colorOf(s.name), label: s.name ?? view.yName, value: formatNumber(s.values[bin] as number) })),
      );
    };
    return option;
  }

  // Scatter: points, or a grid of counts when there are too many.
  const xTime = data.x.time;
  const value = (v: number, time: boolean, timeOnly: boolean) => (time ? formatTime(v, timeOnly) : formatNumber(v));
  const bounds = (range: [number, number] | null, fallback?: [number, number]) => {
    const [min, max] = range ?? fallback ?? [undefined, undefined];
    return { min, max };
  };
  option.xAxis = {
    ...axis,
    type: xTime ? 'time' : 'value',
    scale: true,
    name: view.xName,
    axisLabel: xTime ? timeLabel(data.xTimeOnly) : { ...axis.axisLabel, formatter: boundedNumber(view.xRange ?? data.density?.x ?? null) },
    ...bounds(view.xRange, data.density?.x),
  };
  option.yAxis = { ...yAxis, scale: true, ...bounds(view.yRange, data.density?.y) };
  if (!view.yTime) (option.yAxis as Options).axisLabel = { ...axis.axisLabel, formatter: boundedNumber(view.yRange ?? data.density?.y ?? null) };
  (option.tooltip as Options).trigger = 'item';

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
    ];
    (option.tooltip as Options).formatter = (item: { data: number[] }) => {
      const [x0, x1, y0, y1, n] = item.data;
      return tip(`${formatNumber(n)} ${n === 1 ? 'row' : 'rows'}`, [
        { label: view.xName, value: `${value(x0, xTime, data.xTimeOnly)} to ${value(x1, xTime, data.xTimeOnly)}` },
        { label: view.yName, value: `${value(y0, view.yTime, data.yTimeOnly)} to ${value(y1, view.yTime, data.yTimeOnly)}` },
      ]);
    };
    return option;
  }

  const total = data.series.reduce((n, s) => n + s.points.length / 2, 0);
  // Few points are drawn large enough to hit; many are drawn small and see-through so crowds show.
  const size = view.pointSize > 0 ? view.pointSize : total <= 300 ? 8 : total <= 5_000 ? 6 : 4;
  option.series = data.series.map((s) => {
    const points: number[][] = [];
    for (let i = 0; i < s.points.length; i += 2) points.push([s.points[i], s.points[i + 1]]);
    return {
      type: 'scatter',
      name: s.name ?? view.yName,
      data: points,
      symbolSize: size,
      itemStyle: { color: colorOf(s.name), opacity: total > 5_000 ? 0.55 : 0.85, borderColor: look.bg, borderWidth: total <= 300 ? 1 : 0 },
      emphasis: { scale: 1.6 },
      large: total > 20_000,
      largeThreshold: 20_000,
    };
  });
  (option.tooltip as Options).formatter = (item: { data: number[]; seriesName: string; seriesIndex: number }) =>
    tip(data.series[item.seriesIndex]?.name ?? '', [
      { label: view.xName, value: value(item.data[0], xTime, data.xTimeOnly) },
      { label: view.yName, value: value(item.data[1], view.yTime, data.yTimeOnly) },
    ]);
  return option;
}
