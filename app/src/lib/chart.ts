// What the graph builder asks the backend for and gets back, and the small
// helpers for showing it. The rows of a result never come to the UI for a
// chart: the backend works out the bars, bins or points and sends only those.

import type { ColumnKind } from './types';

/** How a column's values are read: as numbers, as dates and times, or as separate categories. */
export type ChartRole = 'continuous' | 'time' | 'discrete';
export type ChartKind = 'bar' | 'histogram' | 'scatter';
export type Statistic = 'count' | 'sum' | 'mean' | 'min' | 'max';
export type TimeBucket = 'second' | 'minute' | 'hour' | 'day' | 'week' | 'month' | 'quarter' | 'year';

export interface AxisRef {
  column: number;
  role: ChartRole;
}

/**
 * Keeps rows by one column: `min` and `max` for numbers and times, `values`
 * for categories (null stands for NULL; an empty list keeps everything).
 */
export interface ChartFilter {
  column: number;
  role: ChartRole;
  min: number | null;
  max: number | null;
  values: (string | null)[];
}

export interface ChartRequest {
  kind: ChartKind;
  x: AxisRef | null;
  y: AxisRef | null;
  /** A column whose values split the chart into coloured groups. */
  color: number | null;
  statistic: Statistic;
  /** About how many bins; null lets the backend choose. */
  bins: number | null;
  /** The unit time is cut into; null lets the backend choose. */
  bucket: TimeBucket | null;
  filters: ChartFilter[];
  xRange: [number, number] | null;
  yRange: [number, number] | null;
  /** What a scatter plot of too many points becomes. */
  large: 'density' | 'sample';
  sort: 'value' | 'label' | 'seen';
  maxCategories: number | null;
}

export type XAxis =
  | { type: 'categories'; labels: string[] }
  | { type: 'bins'; edges: number[]; time: boolean; bucket: TimeBucket | null }
  | { type: 'continuous'; time: boolean };

export interface ChartSeries {
  /** The colour group, or null when the chart is not split by colour. */
  name: string | null;
  /** One value per category or bin. */
  values: (number | null)[];
  /** Scatter points as x, y, x, y and so on. */
  points: number[];
}

export interface Density {
  cols: number;
  rows: number;
  x: [number, number];
  y: [number, number];
  /** Row by row from the lowest Y. */
  counts: number[];
  max: number;
}

export interface ChartData {
  rowsTotal: number;
  /** Rows that pass the filters and ranges. */
  rowsFiltered: number;
  /** Of those, rows with a usable value on every axis in use. */
  rowsDrawn: number;
  x: XAxis;
  series: ChartSeries[];
  density: Density | null;
  sampled: boolean;
  /** Smallest and largest X under the filters, ignoring the X range. */
  xExtent: [number, number] | null;
  xTimeOnly: boolean;
  yTimeOnly: boolean;
  notes: string[];
}

export interface ColumnSummary {
  rows: number;
  missing: number;
  min: number | null;
  max: number | null;
  timeOnly: boolean;
  distinct: number;
  overflow: boolean;
  categories: { label: string | null; rows: number }[];
}

export const NULL_LABEL = '(null)';
export const OTHER_LABEL = 'Other';

/** How a column is read unless the user says otherwise. Null: not something to chart. */
export function defaultRole(kind: ColumnKind): ChartRole | null {
  switch (kind) {
    case 'number': return 'continuous';
    case 'date': return 'time';
    case 'text': return 'discrete';
    default: return null;
  }
}

/** The ways a column of this kind can be read, the usual one first. */
export function rolesFor(kind: ColumnKind): ChartRole[] {
  switch (kind) {
    case 'number': return ['continuous', 'discrete'];
    case 'date': return ['time', 'discrete'];
    case 'text': return ['discrete', 'continuous', 'time'];
    default: return [];
  }
}

export const ROLE_LABELS: Record<ChartRole, string> = {
  continuous: 'Numbers',
  time: 'Time',
  discrete: 'Categories',
};

export const STATISTIC_LABELS: Record<Statistic, string> = {
  sum: 'Sum',
  mean: 'Average',
  min: 'Smallest',
  max: 'Largest',
  count: 'Count',
};

export const BUCKETS: TimeBucket[] = ['second', 'minute', 'hour', 'day', 'week', 'month', 'quarter', 'year'];

// Series colours. A fixed order chosen so that neighbours stay apart for
// readers who do not see every colour; one set for light backgrounds and the
// same hues stepped for dark ones. `tools/themes.py --check` checks them
// against every theme's background.
export const SERIES_LIGHT = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#6250d6', '#e34948'];
export const SERIES_DARK = ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#0ca30c', '#9085e9', '#e66767'];
/** One hue from pale to deep, for "how many": the shading of a dense scatter plot. */
export const SEQUENTIAL = ['#cde2fb', '#9ec5f4', '#6da7ec', '#3987e5', '#256abf', '#184f95', '#0d366b'];

const pad = (n: number, width = 2) => String(Math.trunc(n)).padStart(width, '0');

/**
 * A time as text. Times are milliseconds since 1970 with no time zone, so
 * they are read back with the UTC accessors: what comes out is what the
 * server sent, whatever this computer's time zone.
 */
export function formatTime(ms: number, timeOnly = false): string {
  if (!Number.isFinite(ms)) return '';
  if (timeOnly) {
    const sign = ms < 0 ? '-' : '';
    const seconds = Math.floor(Math.abs(ms) / 1000);
    return `${sign}${pad(seconds / 3600)}:${pad((seconds / 60) % 60)}:${pad(seconds % 60)}`;
  }
  const d = new Date(ms);
  const date = `${pad(d.getUTCFullYear(), 4)}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`;
  const seconds = d.getUTCSeconds();
  if (d.getUTCHours() === 0 && d.getUTCMinutes() === 0 && seconds === 0) return date;
  return `${date} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}${seconds ? `:${pad(seconds)}` : ''}`;
}

/** What one time bucket is called: "2026", "2026 Q3", "2026-07", a date, or a date and time. */
export function formatBucket(ms: number, bucket: TimeBucket | null, timeOnly = false): string {
  if (timeOnly) return formatTime(ms, true);
  const d = new Date(ms);
  const year = pad(d.getUTCFullYear(), 4);
  switch (bucket) {
    case 'year': return year;
    case 'quarter': return `${year} Q${Math.floor(d.getUTCMonth() / 3) + 1}`;
    case 'month': return `${year}-${pad(d.getUTCMonth() + 1)}`;
    case 'week': return `week of ${formatTime(ms)}`;
    default: return formatTime(ms);
  }
}

/** Reads a date, or a date and time, typed as `2026-07-04` or `2026-07-04 13:30[:15]`. Null if it is not one. */
export function parseTime(text: string): number | null {
  const m = /^\s*(\d{4})-(\d{1,2})-(\d{1,2})(?:[ T](\d{1,2}):(\d{2})(?::(\d{2}))?)?\s*$/.exec(text);
  if (!m) return null;
  const [year, month, day, hour, minute, second] = m.slice(1).map((part) => Number(part ?? 0));
  if (month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 59) return null;
  const ms = Date.UTC(year, month - 1, day, hour, minute, second);
  // Date.UTC reads years 0 to 99 as 1900 to 1999.
  return year < 100 ? new Date(ms).setUTCFullYear(year) : ms;
}

const plain = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 });
const precise = new Intl.NumberFormat('en-US', { maximumSignificantDigits: 4 });

/** A number for a tooltip or a label: thousands separated, no more digits than help. */
export function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return '';
  const size = Math.abs(n);
  if (size !== 0 && (size < 0.01 || size >= 1e15)) return n.toExponential(3);
  return size < 100 && !Number.isInteger(n) ? precise.format(n) : plain.format(n);
}

/** `#rrggbb` between two colours; `share` 0 gives `a`, 1 gives `b`. */
export function mixColors(a: string, b: string, share: number): string {
  const part = (color: string, at: number) => parseInt(color.slice(at, at + 2), 16);
  return '#' + [1, 3, 5].map((at) => Math.round(part(a, at) + (part(b, at) - part(a, at)) * share).toString(16).padStart(2, '0')).join('');
}

/** How light a `#rrggbb` colour is, 0 (black) to 1 (white). */
export function luminance(color: string): number {
  const channel = (at: number) => {
    const c = parseInt(color.slice(at, at + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}
