// What a query tab's graph is set up to show. Kept on the tab, so leaving
// the graph for the query and coming back finds it as it was.

import { defaultRole, rolesFor } from './chart';
import type { AxisRef, ChartFilter, ChartKind, ChartRequest, ChartRole, Statistic, TimeBucket } from './chart';
import type { ColumnInfo } from './types';

export type Zone = 'x' | 'y' | 'color';

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

  // How it is drawn. None of these change what the backend is asked for.
  stacked = $state(true);
  legend = $state<'right' | 'bottom' | 'hidden'>('right');
  title = $state('');
  xLabel = $state('');
  yLabel = $state('');
  logY = $state(false);
  /** Size of a scatter point in pixels; 0 picks one from the number of points. */
  pointSize = $state(0);

  /**
   * The colour each group has been given, so a group keeps its colour when a
   * filter removes or brings back others. Forgotten when the colour column changes.
   */
  private slots = new Map<string, number>();

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

  /** Puts a column on an axis or makes it the colour, read the usual way for its kind. */
  put(zone: Zone, column: number): void {
    const role = defaultRole(this.columns[column].kind);
    if (role === null) return;
    if (zone === 'color') {
      if (this.color !== column) this.slots.clear();
      this.color = column;
      return;
    }
    this[zone] = { column, role };
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
    if (this.kind === 'scatter') return this.x !== null && this.y !== null;
    if (this.kind === 'histogram') return this.x !== null;
    return this.x !== null || this.y !== null || this.color !== null;
  }

  /** What to put where before this kind of chart can be drawn. */
  get hint(): string {
    if (this.kind === 'scatter') return 'Put a number or time column on X and another on Y.';
    if (this.kind === 'histogram') return 'Put a number or time column on X to see how its values are spread.';
    return 'Put a column on X to get a bar for each of its values. Add a number column on Y to add it up instead of counting rows.';
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

  get request(): ChartRequest {
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
      maxCategories: this.maxCategories,
    };
  }
}
