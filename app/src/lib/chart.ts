// What the graph builder asks the backend for and gets back, and the small
// helpers for showing it. The rows of a result never come to the UI for a
// chart: the backend works out the bars, bins or points and sends only those.

import type { ColumnKind } from './types';

/** How a column's values are read: as numbers, as dates and times, or as separate categories. */
export type ChartRole = 'continuous' | 'time' | 'discrete';
export type ChartKind = 'bar' | 'line' | 'pie' | 'pareto' | 'histogram' | 'box' | 'scatter' | 'control';
/** The shape of a trend line. */
export type TrendKind = 'linear' | 'polynomial' | 'exponential' | 'logarithmic' | 'power';

export interface TrendSpec {
  kind: TrendKind;
  /** For a polynomial: 2 to 6. */
  degree: number;
  /** One line for each colour group instead of one for all rows. */
  perGroup: boolean;
}
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
  /** A trend line through a scatter plot, fitted by the backend to every row drawn. */
  trend: TrendSpec | null;
  /** Specification limits of the measured column: rows outside them are counted. */
  specLow: number | null;
  specHigh: number | null;
}

export type XAxis =
  | { type: 'categories'; labels: string[] }
  | { type: 'bins'; edges: number[]; time: boolean; bucket: TimeBucket | null }
  | { type: 'continuous'; time: boolean }
  /** Points numbered from 1 in the order of the result, each standing for `rowsPerPoint` rows. */
  | { type: 'sequence'; count: number; rowsPerPoint: number };

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

/** The five numbers of a box plot, and what lies outside the whiskers. */
export interface BoxStats {
  rows: number;
  min: number;
  q1: number;
  median: number;
  q3: number;
  max: number;
  /** Where the whiskers end. */
  low: number;
  high: number;
  mean: number;
  outliers: number[];
  outlierCount: number;
}

/** The centre line and limits of a control chart. */
export interface Control {
  center: number;
  sigma: number;
  upper: (number | null)[];
  lower: (number | null)[];
  /** Points outside their limits. */
  beyond: number;
  /** Each point is one row. */
  individuals: boolean;
}

/** The measured column in figures, among the rows drawn. */
export interface Stats {
  rows: number;
  mean: number;
  stdDev: number;
  /** The short-term standard deviation, from row-to-row changes. */
  sigmaWithin: number | null;
  min: number;
  max: number;
  below: number | null;
  above: number | null;
}

/**
 * A fitted trend line: v = c0 + c1·t + c2·t² + … where t = (u - center) / scale,
 * u is x (or its logarithm) and v is y (or its logarithm), as the kind says.
 */
export interface Fit {
  series: number | null;
  kind: TrendKind;
  coefficients: number[];
  center: number;
  scale: number;
  rows: number;
  rSquared: number;
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
  /** For a box plot: one entry per category or bin, null where there are no values. */
  boxes: (BoxStats | null)[];
  control: Control | null;
  stats: Stats | null;
  fits: Fit[];
}

/**
 * A line the user has drawn at a value. It belongs to what was measured (a
 * column's values, or a count), not to an axis: a limit set on a column shows
 * wherever that column is a scale, along X in a histogram and up Y in a
 * control chart, and not on a chart of something else.
 */
export interface RefLine {
  id: number;
  /** What the value is a value of: see `GraphState.subject`. */
  subject: string;
  /** The axis it is drawn on in the chart in front. */
  axis: 'x' | 'y';
  /** The value is a time. */
  time: boolean;
  value: number;
  style: 'solid' | 'dashed' | 'dotted';
  color: 'red' | 'green' | 'blue' | 'grey';
  /** Shown beside the line; may be empty. */
  name: string;
  /** What the line means to the figures: a specification limit, a target, or nothing. */
  role: 'none' | 'lsl' | 'usl' | 'target';
}

/** The short names of what a line means, for lists. */
export const LINE_ROLE_NAMES: Record<RefLine['role'], string> = { none: '', lsl: 'LSL', usl: 'USL', target: 'target' };

export const LINE_ROLES: Record<RefLine['role'], string> = {
  none: 'Just a line',
  lsl: 'Lower specification limit (LSL)',
  usl: 'Upper specification limit (USL)',
  target: 'Target',
};

export const TREND_LABELS: Record<TrendKind | 'moving', string> = {
  linear: 'Straight line',
  polynomial: 'Polynomial',
  exponential: 'Exponential',
  logarithmic: 'Logarithmic',
  power: 'Power',
  moving: 'Moving average',
};

/** The kinds of chart, in the order of their buttons. */
export const KINDS: { kind: ChartKind; label: string; hint: string }[] = [
  { kind: 'bar', label: 'Bar', hint: 'A bar for each value of X: a count of rows, or a sum or average of Y' },
  { kind: 'line', label: 'Line', hint: 'The numbers of a bar chart joined by a line: a run chart when X is time' },
  { kind: 'pie', label: 'Pie', hint: 'The share each value of a column has of the whole' },
  { kind: 'pareto', label: 'Pareto', hint: 'Bars from the largest down with a running total, to find the few causes of most of the total' },
  { kind: 'histogram', label: 'Histogram', hint: 'How the values of one number or time column are spread; with specification limits, its capability' },
  { kind: 'box', label: 'Box', hint: 'The median, quartiles and outliers of Y, for each value of X' },
  { kind: 'scatter', label: 'Scatter', hint: 'A point for each row: one number or time column against another' },
  { kind: 'control', label: 'Control', hint: 'Averages of Y in order, against control limits worked out from its own variation' },
];

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
  if (size !== 0 && (size < 1e-4 || size >= 1e15)) return n.toExponential(3);
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

// ---------- trend lines ----------

const logsX = (kind: TrendKind) => kind === 'logarithmic' || kind === 'power';
const logsY = (kind: TrendKind) => kind === 'exponential' || kind === 'power';

/** Solves a small system of equations. Null if the points do not pin the curve down. */
function solve(a: number[][], b: number[]): number[] | null {
  const n = b.length;
  for (let column = 0; column < n; column++) {
    let pivot = column;
    for (let row = column + 1; row < n; row++) if (Math.abs(a[row][column]) > Math.abs(a[pivot][column])) pivot = row;
    if (Math.abs(a[pivot][column]) < 1e-10) return null;
    [a[column], a[pivot]] = [a[pivot], a[column]];
    [b[column], b[pivot]] = [b[pivot], b[column]];
    for (let row = column + 1; row < n; row++) {
      const factor = a[row][column] / a[column][column];
      for (let k = column; k < n; k++) a[row][k] -= factor * a[column][k];
      b[row] -= factor * b[column];
    }
  }
  const c = new Array<number>(n).fill(0);
  for (let row = n - 1; row >= 0; row--) {
    let known = 0;
    for (let k = row + 1; k < n; k++) known += a[row][k] * c[k];
    c[row] = (b[row] - known) / a[row][row];
  }
  return c;
}

/**
 * The least-squares trend line through points, the same way the backend
 * fits one to the rows of a scatter plot. Used for charts whose points are
 * already few (a line chart, bars over time, a control chart). Null if
 * there are too few usable points.
 */
export function fitPoints(xs: number[], ys: number[], kind: TrendKind, degree: number, series: number | null = null): Fit | null {
  const d = kind === 'polynomial' ? Math.min(6, Math.max(2, Math.round(degree))) : 1;
  const us: number[] = [];
  const vs: number[] = [];
  for (let i = 0; i < xs.length; i++) {
    const [x, y] = [xs[i], ys[i]];
    if (!Number.isFinite(x) || !Number.isFinite(y) || (logsX(kind) && x <= 0) || (logsY(kind) && y <= 0)) continue;
    us.push(logsX(kind) ? Math.log(x) : x);
    vs.push(logsY(kind) ? Math.log(y) : y);
  }
  const n = us.length;
  if (n <= d) return null;
  const [uLow, uHigh, vLow, vHigh] = [Math.min(...us), Math.max(...us), Math.min(...vs), Math.max(...vs)];
  const center = (uLow + uHigh) / 2;
  const scale = uHigh > uLow ? (uHigh - uLow) / 2 : 1;
  const middle = (vLow + vHigh) / 2;
  const powers = new Array<number>(2 * d + 1).fill(0);
  const products = new Array<number>(d + 1).fill(0);
  let [wSum, wSquares] = [0, 0];
  for (let i = 0; i < n; i++) {
    const [t, w] = [(us[i] - center) / scale, vs[i] - middle];
    let power = 1;
    for (let k = 0; k <= 2 * d; k++) {
      powers[k] += power;
      if (k <= d) products[k] += w * power;
      power *= t;
    }
    wSum += w;
    wSquares += w * w;
  }
  const coefficients = solve(
    Array.from({ length: d + 1 }, (_, i) => Array.from({ length: d + 1 }, (_, j) => powers[i + j])),
    products.slice(),
  );
  if (!coefficients) return null;
  const explained = coefficients.reduce((sum, c, k) => sum + c * products[k], 0);
  const total = wSquares - (wSum * wSum) / n;
  const rSquared = total > 0 ? Math.min(1, Math.max(0, 1 - (wSquares - explained) / total)) : 1;
  coefficients[0] += middle;
  return { series, kind, coefficients, center, scale, rows: n, rSquared };
}

/** The value a trend line gives at x. NaN where it has none (a logarithm of zero or less). */
export function evalFit(fit: Fit, x: number): number {
  if (logsX(fit.kind) && x <= 0) return NaN;
  const t = ((logsX(fit.kind) ? Math.log(x) : x) - fit.center) / fit.scale;
  let v = 0;
  for (let k = fit.coefficients.length - 1; k >= 0; k--) v = v * t + fit.coefficients[k];
  return logsY(fit.kind) ? Math.exp(v) : v;
}

const sig = (n: number) => {
  if (n === 0 || !Number.isFinite(n)) return '0';
  const size = Math.abs(n);
  return size >= 1e6 || size < 1e-3 ? n.toExponential(3).replace('e+', 'e') : String(Number(n.toPrecision(4)));
};
/** " + 3" or " - 3", for the later terms of an equation. */
const signed = (n: number) => (n < 0 ? ` - ${sig(-n)}` : ` + ${sig(n)}`);

/** How much of the variation a trend line accounts for, as "R² = 0.93". */
export function fitQuality(fit: Fit): string {
  return `R² = ${fit.rSquared.toFixed(fit.rSquared > 0.999 ? 4 : 3)}`;
}

/**
 * A trend line as an equation, with how well it fits. Over time the
 * coefficients would be per millisecond, so a straight or exponential line
 * is given as a change per day and the others by their fit alone.
 */
export function describeFit(fit: Fit, time: boolean): string {
  const c = fit.coefficients;
  // In terms of u itself instead of the centred, scaled t.
  const slope = (c[1] ?? 0) / fit.scale;
  const start = c[0] - ((c[1] ?? 0) * fit.center) / fit.scale;
  const quality = fitQuality(fit);
  const DAY = 86_400_000;
  let equation: string;
  switch (fit.kind) {
    case 'linear':
      equation = time ? `${slope >= 0 ? 'rises' : 'falls'} ${sig(Math.abs(slope * DAY))} a day` : `y = ${sig(slope)}\u00b7x${signed(start)}`;
      break;
    case 'exponential':
      equation = time ? `\u00d7${sig(Math.exp(slope * DAY))} a day` : `y = ${sig(Math.exp(start))}\u00b7e^(${sig(slope)}\u00b7x)`;
      break;
    case 'logarithmic':
      equation = `y = ${sig(slope)}\u00b7ln(x)${signed(start)}`;
      break;
    case 'power':
      equation = `y = ${sig(Math.exp(start))}\u00b7x^${sig(slope)}`;
      break;
    default: {
      if (time) {
        equation = `polynomial of degree ${c.length - 1}`;
        break;
      }
      // Multiply out c0 + c1·t + … with t = (x - center) / scale.
      let plain = [0];
      for (let k = c.length - 1; k >= 0; k--) {
        const next = new Array<number>(plain.length + 1).fill(0);
        for (let i = 0; i < plain.length; i++) {
          next[i] += (plain[i] * -fit.center) / fit.scale;
          next[i + 1] += plain[i] / fit.scale;
        }
        next[0] += c[k];
        plain = next;
      }
      const terms: string[] = [];
      for (let k = c.length - 1; k >= 0; k--) {
        const power = k === 0 ? '' : k === 1 ? '\u00b7x' : `\u00b7x^${k}`;
        terms.push(terms.length === 0 ? `${sig(plain[k])}${power}` : `${signed(plain[k])}${power}`);
      }
      equation = `y = ${terms.join('')}`;
    }
  }
  return `${equation}  (${quality})`;
}

/** The average of each value and the ones before it, `window` values in all. Nulls are skipped and left null. */
export function movingAverage(values: (number | null)[], window: number): (number | null)[] {
  const size = Math.max(2, Math.round(window));
  const recent: number[] = [];
  let sum = 0;
  return values.map((value) => {
    if (value === null) return null;
    recent.push(value);
    sum += value;
    if (recent.length > size) sum -= recent.shift() as number;
    return recent.length === size ? sum / size : null;
  });
}

/** The height of the bell curve of a normal distribution at x. */
export function normalDensity(x: number, mean: number, sigma: number): number {
  return Math.exp(-0.5 * ((x - mean) / sigma) ** 2) / (sigma * Math.sqrt(2 * Math.PI));
}

/** How well a process fits its specification limits. */
export interface Capability {
  /** Limits against the short-term spread: what the process could do if it held its level. */
  cp: number | null;
  cpk: number | null;
  /** Limits against the overall spread: what it did. */
  pp: number | null;
  ppk: number | null;
  /** Rows outside the limits, as a share of all. */
  outside: number | null;
}

export function capability(stats: Stats, low: number | null, high: number | null): Capability | null {
  if (low === null && high === null) return null;
  const index = (sigma: number | null) => {
    if (sigma === null || !(sigma > 0)) return [null, null] as const;
    const sides = [high === null ? null : (high - stats.mean) / (3 * sigma), low === null ? null : (stats.mean - low) / (3 * sigma)].filter(
      (side): side is number => side !== null,
    );
    return [low !== null && high !== null ? (high - low) / (6 * sigma) : null, Math.min(...sides)] as const;
  };
  const [cp, cpk] = index(stats.sigmaWithin);
  const [pp, ppk] = index(stats.stdDev);
  const outside = stats.rows > 0 ? ((stats.below ?? 0) + (stats.above ?? 0)) / stats.rows : null;
  return { cp, cpk, pp, ppk, outside };
}
