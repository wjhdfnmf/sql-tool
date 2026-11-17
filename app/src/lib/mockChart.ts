// Chart summaries for the mock backend, so the graph builder can be worked on
// in a plain browser. A small imitation of `crates/core/src/chart.rs`: the
// same request and the same shape of answer, worked out here from the mock's
// synthetic rows. It reads at most ROW_CAP rows; the real backend reads all.

import { NULL_LABEL, OTHER_LABEL, fitPoints, parseTime } from './chart';
import type { BoxStats, ChartData, ChartRequest, ChartRole, ChartSeries, ColumnSummary, Stats, TimeBucket, XAxis } from './chart';

const ROW_CAP = 200_000;
const MAX_POINTS = 50_000;
const MAX_SERIES = 8;
const MAX_CONTROL_POINTS = 2_000;

type Cell = (row: number, column: number) => string | null;
type Column = { numbers: Float64Array; codes: null; labels: null } | { numbers: null; codes: Int32Array; labels: string[] };

const kept = new Map<string, Column>();
const fail = (message: string) => ({ kind: 'chart', message });
const count = (n: number) => n.toLocaleString('en-US');

/** Forgets what was read out of a result. */
export function mockChartClose(resultId: number): void {
  for (const key of [...kept.keys()]) if (key.startsWith(`${resultId}:`)) kept.delete(key);
}

function readTime(text: string): number {
  const date = parseTime(text.length > 19 ? text.slice(0, 19) : text);
  if (date !== null) return date;
  const m = /^(-?)(\d+):(\d\d)(?::(\d\d))?/.exec(text);
  return m ? (m[1] ? -1 : 1) * ((Number(m[2]) * 60 + Number(m[3])) * 60 + Number(m[4] ?? 0)) * 1000 : NaN;
}

function load(resultId: number, cell: Cell, rows: number, column: number, role: ChartRole): Column {
  const key = `${resultId}:${column}:${role}`;
  const found = kept.get(key);
  if (found) return found;
  let out: Column;
  if (role === 'discrete') {
    const codes = new Int32Array(rows);
    const labels: string[] = [];
    const index = new Map<string, number>();
    for (let row = 0; row < rows; row++) {
      const text = cell(row, column);
      if (text === null) {
        codes[row] = -1;
        continue;
      }
      let code = index.get(text);
      if (code === undefined) {
        code = labels.length;
        labels.push(text.length > 120 ? `${text.slice(0, 120)}…` : text);
        index.set(text, code);
      }
      codes[row] = code;
    }
    out = { numbers: null, codes, labels };
  } else {
    const numbers = new Float64Array(rows);
    for (let row = 0; row < rows; row++) {
      const text = cell(row, column);
      const value = text === null || text.trim() === '' ? NaN : role === 'time' ? readTime(text) : Number(text);
      numbers[row] = Number.isFinite(value) ? value : NaN;
    }
    out = { numbers, codes: null, labels: null };
  }
  kept.set(key, out);
  return out;
}

export function mockChartColumn(resultId: number, cell: Cell, total: number, column: number, role: ChartRole): ColumnSummary {
  const rows = Math.min(total, ROW_CAP);
  const data = load(resultId, cell, rows, column, role);
  const summary: ColumnSummary = { rows, missing: 0, min: null, max: null, timeOnly: false, distinct: 0, overflow: false, categories: [] };
  if (data.numbers) {
    for (const v of data.numbers) {
      if (Number.isNaN(v)) summary.missing++;
      else {
        summary.min = summary.min === null ? v : Math.min(summary.min, v);
        summary.max = summary.max === null ? v : Math.max(summary.max, v);
      }
    }
  } else {
    const counts = new Array<number>(data.labels.length).fill(0);
    for (const code of data.codes) {
      if (code < 0) summary.missing++;
      else counts[code]++;
    }
    summary.distinct = data.labels.length;
    summary.categories = data.labels.map((label, i) => ({ label: label as string | null, rows: counts[i] }));
    if (summary.missing > 0) summary.categories.push({ label: null, rows: summary.missing });
    summary.categories.sort((a, b) => b.rows - a.rows || natural(a.label ?? '', b.label ?? ''));
    summary.categories.length = Math.min(summary.categories.length, 1000);
  }
  return summary;
}

function natural(a: string, b: string): number {
  const [x, y] = [Number(a), Number(b)];
  const [nx, ny] = [a.trim() !== '' && Number.isFinite(x), b.trim() !== '' && Number.isFinite(y)];
  if (nx && ny) return x - y;
  if (nx !== ny) return nx ? -1 : 1;
  return a.toLowerCase().localeCompare(b.toLowerCase());
}

interface Tally {
  count: number;
  sum: number;
  min: number;
  max: number;
}

const empty = (): Tally => ({ count: 0, sum: 0, min: Infinity, max: -Infinity });

function add(t: Tally, v: number): void {
  t.count++;
  t.sum += v;
  t.min = Math.min(t.min, v);
  t.max = Math.max(t.max, v);
}

function merge(into: Tally, t: Tally): void {
  into.count += t.count;
  into.sum += t.sum;
  into.min = Math.min(into.min, t.min);
  into.max = Math.max(into.max, t.max);
}

function value(t: Tally, statistic: ChartRequest['statistic']): number | null {
  if (statistic === 'count') return t.count;
  if (t.count === 0) return null;
  return statistic === 'sum' ? t.sum : statistic === 'mean' ? t.sum / t.count : statistic === 'min' ? t.min : t.max;
}

const UNITS: TimeBucket[] = ['second', 'minute', 'hour', 'day', 'week', 'month', 'quarter', 'year'];
const DAY = 86_400_000;

function bucketIndex(unit: TimeBucket, ms: number): number {
  const d = new Date(ms);
  switch (unit) {
    case 'second': return Math.floor(ms / 1000);
    case 'minute': return Math.floor(ms / 60_000);
    case 'hour': return Math.floor(ms / 3_600_000);
    case 'day': return Math.floor(ms / DAY);
    case 'week': return Math.floor((Math.floor(ms / DAY) + 3) / 7);
    case 'month': return d.getUTCFullYear() * 12 + d.getUTCMonth();
    case 'quarter': return Math.floor((d.getUTCFullYear() * 12 + d.getUTCMonth()) / 3);
    case 'year': return d.getUTCFullYear();
  }
}

function bucketStart(unit: TimeBucket, index: number): number {
  switch (unit) {
    case 'second': return index * 1000;
    case 'minute': return index * 60_000;
    case 'hour': return index * 3_600_000;
    case 'day': return index * DAY;
    case 'week': return (index * 7 - 3) * DAY;
    case 'month': return Date.UTC(Math.floor(index / 12), index % 12, 1);
    case 'quarter': return bucketStart('month', index * 3);
    case 'year': return Date.UTC(index, 0, 1);
  }
}

function niceStep(raw: number): number {
  const power = 10 ** Math.floor(Math.log10(raw));
  const fraction = raw / power;
  return ([1, 2, 2.5, 5].find((n) => fraction <= n + 1e-9) ?? 10) * power;
}

/** Bins of round numbers, or of calendar units, covering lo to hi. */
function binning(lo: number, hi: number, time: boolean, request: ChartRequest, drawn: number) {
  if (time) {
    const spans = (unit: TimeBucket) => bucketIndex(unit, hi) - bucketIndex(unit, lo) + 1;
    const readable = request.kind === 'line' || request.kind === 'control' ? 240 : 60;
    let unit = request.bucket ?? UNITS.find((u) => spans(u) <= readable) ?? 'year';
    while (unit !== 'year' && spans(unit) > 1000) unit = UNITS[UNITS.indexOf(unit) + 1];
    const first = bucketIndex(unit, lo);
    const final = unit;
    return {
      edges: Array.from({ length: spans(unit) + 1 }, (_, i) => bucketStart(final, first + i)),
      binOf: (v: number) => bucketIndex(final, v) - first,
      bucket: unit as TimeBucket | null,
    };
  }
  const target = request.bins ?? Math.min(60, Math.max(10, (Math.ceil(Math.log2(drawn)) + 1) * 2));
  const step = hi > lo ? niceStep((hi - lo) / target) : 1;
  const start = Math.floor(lo / step) * step;
  const bins = Math.floor((hi - start) / step) + 1;
  return {
    edges: Array.from({ length: bins + 1 }, (_, i) => start + i * step),
    binOf: (v: number) => Math.min(bins - 1, Math.max(0, Math.floor((v - start) / step))),
    bucket: null as TimeBucket | null,
  };
}

function statsOf(values: Float64Array, mask: Uint8Array, request: ChartRequest): Stats | null {
  let [rows, sum, min, max, ranges, previous, below, above] = [0, 0, Infinity, -Infinity, 0, NaN, 0, 0];
  for (let row = 0; row < values.length; row++) {
    const v = values[row];
    if (!mask[row] || Number.isNaN(v)) continue;
    rows++;
    sum += v;
    min = Math.min(min, v);
    max = Math.max(max, v);
    if (!Number.isNaN(previous)) ranges += Math.abs(v - previous);
    previous = v;
    if (request.specLow !== null && v < request.specLow) below++;
    if (request.specHigh !== null && v > request.specHigh) above++;
  }
  if (rows === 0) return null;
  const mean = sum / rows;
  let squares = 0;
  for (let row = 0; row < values.length; row++) if (mask[row] && !Number.isNaN(values[row])) squares += (values[row] - mean) ** 2;
  return {
    rows,
    mean,
    stdDev: rows > 1 ? Math.sqrt(squares / (rows - 1)) : 0,
    sigmaWithin: rows > 1 ? ranges / (rows - 1) / 1.128 : null,
    min,
    max,
    below: request.specLow === null ? null : below,
    above: request.specHigh === null ? null : above,
  };
}

function boxOf(values: number[]): BoxStats | null {
  if (values.length === 0) return null;
  values.sort((a, b) => a - b);
  const last = values.length - 1;
  const quantile = (share: number) => {
    const at = last * share;
    const below = Math.floor(at);
    return values[below] + (at - below) * (values[Math.min(last, below + 1)] - values[below]);
  };
  const [q1, median, q3] = [quantile(0.25), quantile(0.5), quantile(0.75)];
  const reach = 1.5 * (q3 - q1);
  const inside = values.filter((v) => v >= q1 - reach && v <= q3 + reach);
  const outside = values.filter((v) => v < q1 - reach || v > q3 + reach);
  const outliers = outside.length <= 200 ? outside : Array.from({ length: 200 }, (_, i) => outside[Math.floor((i * (outside.length - 1)) / 199)]);
  return {
    rows: values.length,
    min: values[0],
    q1,
    median,
    q3,
    max: values[last],
    low: inside[0],
    high: inside[inside.length - 1],
    mean: values.reduce((a, b) => a + b, 0) / values.length,
    outliers,
    outlierCount: outside.length,
  };
}

export function mockChartData(resultId: number, cell: Cell, total: number, request: ChartRequest): ChartData {
  const rows = Math.min(total, ROW_CAP);
  const column = (ref: { column: number; role: ChartRole } | null) => (ref ? load(resultId, cell, rows, ref.column, ref.role) : null);
  const x = column(request.x);
  const y = column(request.y);
  const coloured = !['pie', 'pareto', 'box', 'control'].includes(request.kind);
  const color = request.color === null || !coloured ? null : load(resultId, cell, rows, request.color, 'discrete');
  const time = request.x?.role === 'time';

  const mask = new Uint8Array(rows).fill(1);
  const within = (values: Float64Array, min: number, max: number) => {
    for (let row = 0; row < rows; row++) if (!(values[row] >= min && values[row] <= max)) mask[row] = 0;
  };
  for (const filter of request.filters) {
    const data = load(resultId, cell, rows, filter.column, filter.role);
    if (data.numbers) {
      if (filter.min !== null || filter.max !== null) within(data.numbers, filter.min ?? -Infinity, filter.max ?? Infinity);
    } else if (filter.values.length > 0) {
      const wanted = new Set(filter.values);
      for (let row = 0; row < rows; row++) {
        const code = data.codes[row];
        if (!wanted.has(code < 0 ? null : data.labels[code])) mask[row] = 0;
      }
    }
  }
  let xExtent: [number, number] | null = null;
  if (x?.numbers) {
    for (let row = 0; row < rows; row++) {
      const v = x.numbers[row];
      if (mask[row] && !Number.isNaN(v)) xExtent = xExtent ? [Math.min(xExtent[0], v), Math.max(xExtent[1], v)] : [v, v];
    }
    if (request.xRange) within(x.numbers, request.xRange[0], request.xRange[1]);
  }
  if (request.kind === 'scatter' && y?.numbers && request.yRange) within(y.numbers, request.yRange[0], request.yRange[1]);

  const data: ChartData = {
    rowsTotal: rows,
    rowsFiltered: mask.reduce((n, m) => n + m, 0),
    rowsDrawn: 0,
    x: { type: 'categories', labels: [] },
    series: [],
    density: null,
    sampled: false,
    xExtent,
    xTimeOnly: false,
    yTimeOnly: false,
    notes: total > ROW_CAP ? [`The mock backend reads only the first ${count(ROW_CAP)} rows.`] : [],
    boxes: [],
    control: null,
    stats: null,
    fits: [],
  };

  // Colour groups: at most MAX_SERIES, the last being Other.
  let names: (string | null)[] = [null];
  let seriesOf: (row: number) => number = () => 0;
  if (color?.codes) {
    const counts = new Map<number, number>();
    for (let row = 0; row < rows; row++) if (mask[row]) counts.set(color.codes[row], (counts.get(color.codes[row]) ?? 0) + 1);
    const name = (code: number) => (code < 0 ? NULL_LABEL : color.labels[code]);
    let present = [...counts.keys()];
    const merged = present.length > MAX_SERIES;
    if (merged) present = present.sort((a, b) => counts.get(b)! - counts.get(a)!).slice(0, MAX_SERIES - 1);
    present.sort((a, b) => (a < 0 ? 1 : b < 0 ? -1 : natural(name(a), name(b))));
    names = present.map(name);
    const slot = new Map(present.map((code, i) => [code, i]));
    if (merged) {
      names.push(OTHER_LABEL);
      data.notes.push(`The colour column has more than ${MAX_SERIES} values; the ${MAX_SERIES - 1} with the most rows are shown and the rest are under ${OTHER_LABEL}.`);
    }
    const other = names.length - 1;
    seriesOf = (row) => slot.get(color.codes[row]) ?? other;
  }
  const series = names.length;

  if (request.kind === 'scatter') {
    if (!x?.numbers || !y?.numbers) throw fail('A scatter plot needs a number or time column on X and on Y.');
    data.x = { type: 'continuous', time };
    const usable: number[] = [];
    let [x0, x1, y0, y1] = [Infinity, -Infinity, Infinity, -Infinity];
    for (let row = 0; row < rows; row++) {
      if (!mask[row] || Number.isNaN(x.numbers[row]) || Number.isNaN(y.numbers[row])) continue;
      usable.push(row);
      x0 = Math.min(x0, x.numbers[row]);
      x1 = Math.max(x1, x.numbers[row]);
      y0 = Math.min(y0, y.numbers[row]);
      y1 = Math.max(y1, y.numbers[row]);
    }
    data.rowsDrawn = usable.length;
    if (request.trend && usable.length > 0) {
      const lines = request.trend.perGroup ? series : 1;
      for (let line = 0; line < lines; line++) {
        const members = request.trend.perGroup ? usable.filter((row) => seriesOf(row) === line) : usable;
        const fit = fitPoints(
          members.map((row) => x.numbers[row]),
          members.map((row) => y.numbers[row]),
          request.trend.kind,
          request.trend.degree,
          request.trend.perGroup ? line : null,
        );
        if (fit) data.fits.push(fit);
        else if (members.length > 0) data.notes.push('There are too few different points to fit that trend line.');
      }
    }
    if (usable.length > MAX_POINTS && request.large === 'density') {
      const [cols, lines] = [160, 100];
      const counts = new Array<number>(cols * lines).fill(0);
      for (const row of usable) {
        const col = Math.min(cols - 1, Math.floor(((x.numbers[row] - x0) / (x1 - x0 || 1)) * cols));
        const line = Math.min(lines - 1, Math.floor(((y.numbers[row] - y0) / (y1 - y0 || 1)) * lines));
        counts[line * cols + col]++;
      }
      data.density = { cols, rows: lines, x: [x0, x1], y: [y0, y1], counts, max: counts.reduce((a, b) => Math.max(a, b), 0) };
      data.notes.push(`${count(usable.length)} points are too many to draw one by one. The shading shows how many fall in each cell; zoom in, or filter, to see the points.`);
      return data;
    }
    const keep = usable.length > MAX_POINTS ? MAX_POINTS / usable.length : 1;
    data.sampled = keep < 1;
    data.series = names.map((name): ChartSeries => ({ name, values: [], points: [] }));
    let shown = 0;
    for (const row of usable) {
      if (keep < 1 && (Math.imul(row, 2654435761) >>> 0) / 4294967296 > keep) continue;
      data.series[seriesOf(row)].points.push(x.numbers[row], y.numbers[row]);
      shown++;
    }
    if (data.sampled) data.notes.push(`A random sample of ${count(shown)} of ${count(usable.length)} points is drawn.`);
    return data;
  }

  if (request.kind === 'box' || request.kind === 'control') {
    const inOrder = request.kind === 'control';
    if (!y?.numbers) throw fail(inOrder ? 'A control chart needs a number column on Y.' : 'A box plot needs a number or time column on Y.');
    const yv = y.numbers;
    const usable = (row: number) => mask[row] === 1 && !Number.isNaN(yv[row]);
    const group = new Int32Array(rows).fill(-1);
    let groups = 0;
    let axis: XAxis;
    if (!x) {
      if (inOrder) {
        let drawn = 0;
        for (let row = 0; row < rows; row++) if (usable(row)) drawn++;
        const size = Math.max(1, Math.ceil(drawn / MAX_CONTROL_POINTS));
        let index = 0;
        for (let row = 0; row < rows; row++) if (usable(row)) group[row] = Math.floor(index++ / size);
        groups = Math.ceil(drawn / size);
        if (size > 1) data.notes.push(`Each point is the average of ${count(size)} rows in a row, in the order of the result.`);
        axis = { type: 'sequence', count: groups, rowsPerPoint: size };
      } else {
        for (let row = 0; row < rows; row++) if (usable(row)) group[row] = 0;
        groups = 1;
        axis = { type: 'categories', labels: ['All rows'] };
      }
    } else if (x.codes) {
      const counts = new Map<number, number>();
      for (let row = 0; row < rows; row++) if (usable(row)) counts.set(x.codes[row], (counts.get(x.codes[row]) ?? 0) + 1);
      const name = (code: number) => (code < 0 ? NULL_LABEL : x.labels[code]);
      let shown = [...counts.keys()];
      const limit = inOrder ? MAX_CONTROL_POINTS : Math.min(500, Math.max(1, request.maxCategories ?? 40));
      let merged = false;
      if (shown.length > limit) {
        if (inOrder) throw fail(`The X column has more than ${count(MAX_CONTROL_POINTS)} different values, too many to be the points of a control chart. Put a time column on X, or leave X empty to chart the rows in order.`);
        data.notes.push(`${count(shown.length)} categories: the ${limit} with the most rows are shown and the rest are under ${OTHER_LABEL}.`);
        shown = shown.sort((a, b) => counts.get(b)! - counts.get(a)!).slice(0, limit);
        merged = true;
      }
      const byName = (a: number, b: number) => (a < 0 ? 1 : b < 0 ? -1 : natural(name(a), name(b)));
      const sort = inOrder ? 'seen' : request.sort;
      if (sort === 'value') shown.sort((a, b) => counts.get(b)! - counts.get(a)! || byName(a, b));
      else if (sort === 'label') shown.sort(byName);
      else shown.sort((a, b) => (a < 0 ? 1 : b < 0 ? -1 : a - b));
      const place = new Map(shown.map((code, i) => [code, i]));
      for (let row = 0; row < rows; row++) if (usable(row)) group[row] = place.get(x.codes[row]) ?? shown.length;
      const labels = shown.map(name);
      if (merged) labels.push(OTHER_LABEL);
      groups = labels.length;
      axis = { type: 'categories', labels };
    } else {
      let [lo, hi, drawn] = [Infinity, -Infinity, 0];
      for (let row = 0; row < rows; row++) {
        if (!usable(row) || Number.isNaN(x.numbers[row])) continue;
        lo = Math.min(lo, x.numbers[row]);
        hi = Math.max(hi, x.numbers[row]);
        drawn++;
      }
      if (drawn === 0) {
        data.x = { type: 'bins', edges: [], time, bucket: null };
        return data;
      }
      const bins = binning(lo, hi, time, request, drawn);
      for (let row = 0; row < rows; row++) if (usable(row) && !Number.isNaN(x.numbers[row])) group[row] = bins.binOf(x.numbers[row]);
      groups = bins.edges.length - 1;
      axis = { type: 'bins', edges: bins.edges, time, bucket: bins.bucket };
    }
    data.x = axis;
    const members: number[][] = Array.from({ length: groups }, () => []);
    for (let row = 0; row < rows; row++) if (group[row] >= 0) members[group[row]].push(yv[row]);
    data.rowsDrawn = members.reduce((n, m) => n + m.length, 0);
    if (!inOrder) {
      data.boxes = members.map(boxOf);
      return data;
    }
    data.stats = statsOf(yv, mask, request);
    if (data.rowsDrawn === 0) return data;
    const means = members.map((m) => (m.length ? m.reduce((a, b) => a + b, 0) / m.length : null));
    const center = members.reduce((sum, m) => sum + m.reduce((a, b) => a + b, 0), 0) / data.rowsDrawn;
    const individuals = members.every((m) => m.length <= 1);
    let sigma = 0;
    if (individuals) {
      const points = means.filter((m): m is number => m !== null);
      for (let i = 1; i < points.length; i++) sigma += Math.abs(points[i] - points[i - 1]);
      sigma = points.length > 1 ? sigma / (points.length - 1) / 1.128 : 0;
    } else {
      let [inside, freedom] = [0, 0];
      members.forEach((m, i) => {
        for (const v of m) inside += (v - (means[i] as number)) ** 2;
        if (m.length >= 2) freedom += m.length - 1;
      });
      sigma = freedom > 0 ? Math.sqrt(inside / freedom) : 0;
    }
    const limit = (n: number, sign: number) => (n > 0 ? center + (sign * 3 * sigma) / Math.sqrt(n) : null);
    const upper = members.map((m) => limit(m.length, 1));
    const lower = members.map((m) => limit(m.length, -1));
    data.series = [{ name: null, values: means, points: [] }];
    data.control = {
      center,
      sigma,
      upper,
      lower,
      beyond: means.filter((m, i) => m !== null && (m > (upper[i] as number) || m < (lower[i] as number))).length,
      individuals,
    };
    return data;
  }

  if (request.kind === 'histogram' && !x?.numbers) throw fail('A histogram needs a number or time column on X.');
  if ((request.kind === 'pie' || request.kind === 'pareto') && !x?.codes) {
    throw fail(`A ${request.kind === 'pie' ? 'pie' : 'Pareto'} chart needs a column on X, read as categories.`);
  }
  const statistic = request.kind === 'histogram' || !y?.numbers ? 'count' : request.statistic;
  const yAt = (row: number): number | null => {
    if (request.kind === 'histogram' || !y) return 1;
    if (y.numbers) return Number.isNaN(y.numbers[row]) ? null : y.numbers[row];
    return y.codes[row] < 0 ? null : 1;
  };

  if (x?.numbers) {
    if (request.kind === 'histogram') data.stats = statsOf(x.numbers, mask, request);
    let [lo, hi, drawn] = [Infinity, -Infinity, 0];
    for (let row = 0; row < rows; row++) {
      if (!mask[row] || Number.isNaN(x.numbers[row]) || yAt(row) === null) continue;
      lo = Math.min(lo, x.numbers[row]);
      hi = Math.max(hi, x.numbers[row]);
      drawn++;
    }
    data.rowsDrawn = drawn;
    if (drawn === 0) {
      data.x = { type: 'bins', edges: [], time, bucket: null };
      return data;
    }
    const { edges, binOf, bucket } = binning(lo, hi, time, request, drawn);
    const bins = edges.length - 1;
    const tallies = Array.from({ length: bins * series }, empty);
    for (let row = 0; row < rows; row++) {
      const v = yAt(row);
      if (!mask[row] || Number.isNaN(x.numbers[row]) || v === null) continue;
      add(tallies[binOf(x.numbers[row]) * series + seriesOf(row)], v);
    }
    data.x = { type: 'bins', edges, time, bucket };
    data.series = names.map((name, s) => ({ name, values: Array.from({ length: bins }, (_, bin) => value(tallies[bin * series + s], statistic)), points: [] }));
    return data;
  }

  // Bars, slices or points over categories, or one bar for everything.
  const labels = x?.codes ? x.labels : ['All rows'];
  const nullAt = labels.length;
  const tallies = Array.from({ length: (labels.length + 1) * series }, empty);
  for (let row = 0; row < rows; row++) {
    const v = yAt(row);
    if (!mask[row] || v === null) continue;
    const code = x?.codes ? x.codes[row] : 0;
    add(tallies[(code < 0 ? nullAt : code) * series + seriesOf(row)], v);
    data.rowsDrawn++;
  }
  const totalOf = (category: number) => {
    const all = empty();
    for (let s = 0; s < series; s++) merge(all, tallies[category * series + s]);
    return all;
  };
  const name = (category: number) => (category === nullAt ? NULL_LABEL : labels[category]);
  let shown = Array.from({ length: labels.length + 1 }, (_, c) => ({ c, t: totalOf(c) })).filter((entry) => entry.t.count > 0);
  const byValue = (a: (typeof shown)[number], b: (typeof shown)[number]) =>
    (value(b.t, statistic) ?? -Infinity) - (value(a.t, statistic) ?? -Infinity) || natural(name(a.c), name(b.c));
  const limit = Math.min(500, Math.max(1, request.maxCategories ?? 40));
  let rest: number[] = [];
  if (shown.length > limit) {
    data.notes.push(`${count(shown.length)} categories: the ${limit} largest are shown and the rest are under ${OTHER_LABEL}.`);
    shown.sort(byValue);
    rest = shown.slice(limit).map((entry) => entry.c);
    shown = shown.slice(0, limit);
  }
  const sort = request.kind === 'pie' || request.kind === 'pareto' ? 'value' : request.sort;
  if (sort === 'value') shown.sort(byValue);
  else if (sort === 'label') shown.sort((a, b) => (a.c === nullAt ? 1 : b.c === nullAt ? -1 : natural(name(a.c), name(b.c))));
  else shown.sort((a, b) => a.c - b.c);
  data.x = { type: 'categories', labels: [...shown.map((entry) => name(entry.c)), ...(rest.length ? [OTHER_LABEL] : [])] };
  data.series = names.map((seriesName, s) => {
    const values = shown.map((entry) => value(tallies[entry.c * series + s], statistic));
    if (rest.length) {
      const all = empty();
      for (const c of rest) merge(all, tallies[c * series + s]);
      values.push(value(all, statistic));
    }
    return { name: seriesName, values, points: [] };
  });
  return data;
}
