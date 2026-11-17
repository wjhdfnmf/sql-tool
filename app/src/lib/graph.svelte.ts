// What a query tab's graph is set up to show. Kept on the tab, so leaving
// the graph for the query and coming back finds it as it was.

import { defaultRole, rolesFor } from './chart';
import type { AxisRef, ChartFilter, ChartKind, ChartRequest, ChartRole, RefLine, Statistic, TimeBucket, TrendKind } from './chart';
import type { ColumnInfo } from './types';

export type Zone = 'x' | 'y' | 'color';

/** A trend line as the user set it up. */
export interface TrendSetup {
  kind: TrendKind | 'moving';
  /** For a polynomial: 2 to 6. */
  degree: number;
  /** For a moving average: how many points are averaged. */
  window: number;
  /** One line for each colour group. */
  perGroup: boolean;
  /** Write the equation and R² on the chart. */
  showEquation: boolean;
}

/** One of the graph builder's popups, and what to do with its answer. */
export type GraphDialogRequest =
  | {
      type: 'line';
      line: RefLine;
      /** The axes a line can go on in this chart. */
      axes: ('x' | 'y')[];
      /** Which of them hold times, so the value is typed and shown as one. */
      time: { x: boolean; y: boolean };
      /** The axis whose lines can be specification limits, if the chart has one. */
      measured: 'x' | 'y' | null;
      save: (line: RefLine) => void;
      /** Set when an existing line is being changed. */
      remove?: () => void;
    }
  | {
      type: 'trend';
      setup: TrendSetup | null;
      /** Whether a moving average, and a line for each colour group, are possible on this chart. */
      moving: boolean;
      perGroup: boolean;
      save: (setup: TrendSetup) => void;
      remove?: () => void;
    }
  | { type: 'text'; heading: string; label: string; value: string; placeholder: string; save: (text: string) => void };

export class GraphState {
  /** The result being charted. */
  resultId = $state(0);
  columns = $state.raw<ColumnInfo[]>([]);

  kind = $state<ChartKind>('bar');
  x = $state<AxisRef | null>(null);
  y = $state<AxisRef | null>(null);
  color = $state<number | null>(null);
  statistic = $state<Statistic>('sum');
  bins = $state<number | null>(null);
  bucket = $state<TimeBucket | null>(null);
  filters = $state<ChartFilter[]>([]);
  /** The part of X shown: the time range, or where the chart is zoomed in to. */
  xRange = $state<[number, number] | null>(null);
  yRange = $state<[number, number] | null>(null);
  large = $state<'density' | 'sample'>('density');
  sort = $state<'value' | 'label' | 'seen'>('value');
  maxCategories = $state(40);

  // How it is drawn.
  stacked = $state(true);
  legend = $state<'right' | 'bottom' | 'hidden'>('right');
  title = $state('');
  xLabel = $state('');
  yLabel = $state('');
  logY = $state(false);
  /** Size of a scatter point in pixels; 0 picks one from the number of points. */
  pointSize = $state(0);
  /** Lines drawn across the chart at chosen values: limits, targets, marks. */
  lines = $state<RefLine[]>([]);
  trend = $state<TrendSetup | null>(null);
  /** Draw the bell curve of a normal distribution with the data's mean and spread over a histogram. */
  normalCurve = $state(false);

  /**
   * The colour each group has been given, so a group keeps its colour when a
   * filter removes or brings back others. Forgotten when the colour column changes.
   */
  private slots = new Map<string, number>();
  private nextLine = 1;

  constructor(resultId: number, columns: ColumnInfo[]) {
    this.resultId = resultId;
    this.columns = columns;
  }

  /** True if these are the columns the graph was set up for (so it can follow a query that was run again). */
  fits(columns: ColumnInfo[]): boolean {
    return columns.length === this.columns.length && columns.every((c, i) => c.name === this.columns[i].name && c.kind === this.columns[i].kind);
  }

  /** Moves the graph to a newer result with the same columns. */
  follow(resultId: number, columns: ColumnInfo[]): void {
    this.resultId = resultId;
    this.columns = columns;
    // The range belonged to the old data.
    this.xRange = null;
    this.yRange = null;
  }

  name(column: number | null | undefined): string {
    return column === null || column === undefined ? '' : (this.columns[column]?.name ?? '');
  }

  /** Columns that can be charted at all: not JSON, not binary. */
  get chartable(): { index: number; column: ColumnInfo }[] {
    return this.columns.map((column, index) => ({ index, column })).filter(({ column }) => defaultRole(column.kind) !== null);
  }

  /** A pie and a Pareto chart are of categories: whatever goes on their X is read as such. */
  private get categorical(): boolean {
    return this.kind === 'pie' || this.kind === 'pareto';
  }

  /** Puts a column on an axis or makes it the colour, read the usual way for its kind and for this chart. */
  put(zone: Zone, column: number): void {
    const role = defaultRole(this.columns[column].kind);
    if (role === null) return;
    if (zone === 'color') {
      if (this.color !== column) this.slots.clear();
      this.color = column;
      return;
    }
    this[zone] = { column, role: zone === 'x' && this.categorical ? 'discrete' : role };
    if (zone === 'x') {
      this.xRange = null;
      this.bins = null;
      this.bucket = null;
    } else {
      this.yRange = null;
    }
  }

  clear(zone: Zone): void {
    if (zone === 'color') {
      this.color = null;
      this.slots.clear();
    } else {
      this[zone] = null;
      if (zone === 'x') this.xRange = null;
      else this.yRange = null;
    }
  }

  setRole(zone: 'x' | 'y', role: ChartRole): void {
    const axis = this[zone];
    if (!axis || axis.role === role) return;
    this[zone] = { column: axis.column, role };
    if (zone === 'x') this.xRange = null;
    else this.yRange = null;
  }

  /** Changes the kind of chart, re-reading the columns in place where the new kind needs them read differently. */
  setKind(kind: ChartKind): void {
    this.kind = kind;
    const usual = (axis: AxisRef | null) => (axis ? defaultRole(this.columns[axis.column]?.kind ?? 'binary') : null);
    if (this.x) {
      if (this.categorical) this.setRole('x', 'discrete');
      // A number column that a pie read as categories goes back to numbers for a chart that needs them.
      else if ((kind === 'histogram' || kind === 'scatter') && this.x.role === 'discrete' && usual(this.x) !== 'discrete') {
        this.setRole('x', usual(this.x) as ChartRole);
      }
    }
    if (this.y && (kind === 'scatter' || kind === 'box' || kind === 'control') && this.y.role === 'discrete' && usual(this.y) !== 'discrete') {
      this.setRole('y', usual(this.y) as ChartRole);
    }
  }

  rolesOf(column: number): ChartRole[] {
    return rolesFor(this.columns[column]?.kind ?? 'binary');
  }

  addFilter(column: number): void {
    if (this.filters.some((f) => f.column === column)) return;
    const role = defaultRole(this.columns[column].kind);
    if (role !== null) this.filters.push({ column, role, min: null, max: null, values: [] });
  }

  removeFilter(column: number): void {
    this.filters = this.filters.filter((f) => f.column !== column);
  }

  /** Whether there is enough on the axes to draw this kind of chart. */
  get ready(): boolean {
    switch (this.kind) {
      case 'scatter': return this.x !== null && this.y !== null;
      case 'histogram':
      case 'pie':
      case 'pareto': return this.x !== null;
      case 'box':
      case 'control': return this.y !== null;
      default: return this.x !== null || this.y !== null || this.color !== null;
    }
  }

  /** What to put where before this kind of chart can be drawn. */
  get hint(): string {
    switch (this.kind) {
      case 'scatter': return 'Put a number or time column on X and another on Y.';
      case 'histogram': return 'Put a number or time column on X to see how its values are spread.';
      case 'pie': return 'Put a column on X to see the share each of its values has. Add a number column on Y to share out its sum instead of counting rows.';
      case 'pareto': return 'Put a column on X (a defect, a cause, a machine) to rank its values from the most rows down. Add a number column on Y to rank by its sum instead.';
      case 'box': return 'Put a number column on Y to see its median, quartiles and outliers. Add a column on X for a box for each of its values.';
      case 'control': return 'Put the measurement on Y. Leave X empty to chart the rows in order, or put a time, a lot or a batch on X to chart its averages.';
      case 'line': return 'Put a time or number column on X and a number column on Y to follow it along X.';
      default: return 'Put a column on X to get a bar for each of its values. Add a number column on Y to add it up instead of counting rows.';
    }
  }

  /** The colour slot of a named group, given the next free one the first time it is seen. */
  slot(name: string): number {
    let slot = this.slots.get(name);
    if (slot === undefined) {
      slot = this.slots.size;
      this.slots.set(name, slot);
    }
    return slot;
  }

  // ---- lines across the chart ----

  /**
   * The axis that holds the measurement, for charts that have one: the one
   * specification limits are set on. Null for charts that only count or compare.
   */
  get measured(): 'x' | 'y' | null {
    return this.kind === 'histogram' ? 'x' : this.kind === 'control' || this.kind === 'box' ? 'y' : null;
  }

  /** X is a scale of numbers or time on this chart, not a row of names. */
  get xScale(): boolean {
    return this.x !== null && this.x.role !== 'discrete' && ['bar', 'line', 'histogram', 'scatter'].includes(this.kind);
  }

  /**
   * What an axis measures on this chart, as a key lines are filed under:
   * the values of a column ("v:3"), its sum ("sum:3"), or a count of rows
   * ("n"). Null where no line can be drawn (a pie, a row of names along X).
   * An average, a smallest and a largest are in the column's own units, so
   * they share its key: a limit on a column shows on a chart of its averages.
   */
  subject(axis: 'x' | 'y'): string | null {
    if (this.kind === 'pie') return null;
    if (axis === 'x') return this.xScale && this.x ? `v:${this.x.column}` : null;
    if (this.kind === 'scatter' || this.kind === 'box' || this.kind === 'control') return this.y ? `v:${this.y.column}` : null;
    if (this.kind === 'histogram' || !this.y || this.y.role !== 'continuous' || this.statistic === 'count') return 'n';
    return this.statistic === 'sum' ? `sum:${this.y.column}` : `v:${this.y.column}`;
  }

  /** The lines that belong on the chart in front, each with the axis it goes on. */
  get shownLines(): RefLine[] {
    const [x, y] = [this.subject('x'), this.subject('y')];
    return this.lines.flatMap((line): RefLine[] => (line.subject === x ? [{ ...line, axis: 'x' }] : line.subject === y ? [{ ...line, axis: 'y' }] : []));
  }

  /** A line with nothing set yet, for the dialog that adds one. */
  newLine(axis: 'x' | 'y', value: number): RefLine {
    return { id: 0, subject: this.subject(axis) ?? '', axis, time: this[axis]?.role === 'time' && this.subject(axis)?.startsWith('v:') === true, value, style: 'dashed', color: 'red', name: '', role: 'none' };
  }

  /** Adds the line, or replaces the one with the same id. It is filed under what its axis measures now. */
  saveLine(line: RefLine): void {
    const filed: RefLine = { ...line, subject: this.subject(line.axis) ?? line.subject, time: this[line.axis]?.role === 'time' && this.subject(line.axis)?.startsWith('v:') === true };
    if (line.id === 0) this.lines.push({ ...filed, id: this.nextLine++ });
    else this.lines = this.lines.map((l) => (l.id === line.id ? filed : l));
  }

  removeLine(id: number): void {
    this.lines = this.lines.filter((l) => l.id !== id);
  }

  /** The specification limit of a kind on the measured axis, if one is drawn. */
  spec(role: 'lsl' | 'usl' | 'target'): number | null {
    const axis = this.measured;
    return this.shownLines.find((l) => l.axis === axis && l.role === role)?.value ?? null;
  }

  get request(): ChartRequest {
    const trend = this.trend;
    return {
      kind: this.kind,
      x: this.x && { ...this.x },
      y: this.kind === 'histogram' ? null : this.y && { ...this.y },
      color: this.color,
      statistic: this.statistic,
      bins: this.bins,
      bucket: this.bucket,
      filters: this.filters.map((f) => ({ column: f.column, role: f.role, min: f.min, max: f.max, values: [...f.values] })),
      xRange: this.xRange && [this.xRange[0], this.xRange[1]],
      yRange: this.kind === 'scatter' && this.yRange ? [this.yRange[0], this.yRange[1]] : null,
      large: this.large,
      sort: this.sort,
      // A pie has eight colours to tell slices apart; more than that go into Other.
      maxCategories: this.kind === 'pie' ? Math.min(this.maxCategories, 8) : this.maxCategories,
      // The backend fits the line of a scatter plot, from every row. Other charts have few points, and are fitted here.
      trend: this.kind === 'scatter' && trend && trend.kind !== 'moving' ? { kind: trend.kind, degree: trend.degree, perGroup: trend.perGroup } : null,
      specLow: this.measured ? this.spec('lsl') : null,
      specHigh: this.measured ? this.spec('usl') : null,
    };
  }
}
