// Virtual result grid, painted on a canvas.
//
// The page never holds the result set. This keeps a small cache of row
// pages fetched from the backend as raw bytes and paints only the cells on
// screen. A transparent scrolling layer sits on top of the canvas and owns
// the native scrollbars, mouse and keyboard input.

import type { ColumnInfo } from './types';

const ROW_H = 24;
const HEADER_H = 28;
const PAD = 8;
const PAGE = 512; // rows per fetched page
const MAX_PAGES = 64; // pages kept in the cache
const MIN_COL_W = 48;
// Browsers cap element height, so the scrollbar's spacer is capped and the
// scroll position is mapped onto the real content height.
const MAX_SCROLL_PX = 8_000_000;
const FONT = '13px system-ui, "Segoe UI", sans-serif';

type CellText = string | null | undefined; // null = SQL NULL, undefined = not loaded

interface Page {
  first: number;
  count: number;
  u8: Uint8Array;
  starts: Uint32Array;
  lens: Int32Array;
  text: CellText[];
  used: number;
}

export type FetchRows = (offset: number, count: number) => Promise<Uint8Array>;

export interface GridCallbacks {
  /** The selected cell changed. */
  onSelect?(row: number, column: number): void;
  /** A cell was double-clicked or Enter was pressed on it. */
  onActivate?(row: number, column: number): void;
  /** Ctrl/Cmd+C on the selected cell. */
  onCopy?(row: number, column: number): void;
  /** A column header was clicked. */
  onSort?(column: number): void;
  /** A cell was right-clicked; x and y are viewport coordinates. */
  onContext?(row: number, column: number, x: number, y: number): void;
}

/** Changes not yet saved, painted over the stored values. */
export interface GridOverlay {
  /** Key `row:column`; null means the cell will be set to NULL. */
  cells: Map<string, string | null>;
  /** Rows that will be deleted. */
  deletedRows: Set<number>;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export class GridView {
  selected: { row: number; column: number } | null = null;

  private readonly ctx: CanvasRenderingContext2D;
  private readonly decoder = new TextDecoder('utf-8');
  private columns: ColumnInfo[] = [];
  private widths: number[] = [];
  /** Left edge of each column in content coordinates, plus the total width. */
  private edges: number[] = [0];
  private rows = 0;
  private top = 0; // scroll offset in content pixels (can exceed MAX_SCROLL_PX)
  private left = 0;
  private width = 0;
  private height = 0;
  private gutter = 56;
  private pages = new Map<number, Page>();
  private inflight = new Set<number>();
  private tick = 0;
  private generation = 0;
  private expectedScrollTop = -1;
  private framePending = false;
  private destroyed = false;
  // Set in the constructor: reading colours needs the canvas, which is not
  // assigned yet when field initialisers run.
  private colors!: ReturnType<GridView['readColors']>;
  private drag: { column: number; startX: number; startWidth: number } | null = null;
  private sort: { column: number; descending: boolean } | null = null;
  private overlay: GridOverlay = { cells: new Map(), deletedRows: new Set() };
  private readonly resizeObserver: ResizeObserver;
  private readonly cleanup: (() => void)[] = [];

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly scroller: HTMLElement,
    private readonly spacer: HTMLElement,
    private readonly fetchRows: FetchRows,
    private readonly callbacks: GridCallbacks = {},
  ) {
    this.ctx = canvas.getContext('2d', { alpha: false })!;
    this.colors = this.readColors();

    const on = <K extends keyof HTMLElementEventMap>(
      target: HTMLElement | Window | MediaQueryList,
      type: K | 'change',
      handler: (e: any) => void,
      options?: AddEventListenerOptions,
    ) => {
      target.addEventListener(type, handler, options);
      this.cleanup.push(() => target.removeEventListener(type, handler, options));
    };

    on(scroller, 'scroll', () => this.onScroll());
    on(scroller, 'wheel', (e: WheelEvent) => this.onWheel(e), { passive: false });
    on(scroller, 'mousedown', (e: MouseEvent) => this.onMouseDown(e));
    on(scroller, 'mousemove', (e: MouseEvent) => this.onHover(e));
    on(scroller, 'dblclick', (e: MouseEvent) => this.onDoubleClick(e));
    on(scroller, 'contextmenu', (e: MouseEvent) => this.onContextMenu(e));
    on(scroller, 'keydown', (e: KeyboardEvent) => this.onKeyDown(e));
    on(window, 'mousemove', (e: MouseEvent) => this.onDragMove(e));
    on(window, 'mouseup', () => (this.drag = null));
    on(window.matchMedia('(prefers-color-scheme: dark)'), 'change', () => {
      this.colors = this.readColors();
      this.invalidate();
    });

    this.resizeObserver = new ResizeObserver(() => this.invalidate());
    this.resizeObserver.observe(scroller);
    this.invalidate();
  }

  destroy(): void {
    this.destroyed = true;
    this.resizeObserver.disconnect();
    for (const undo of this.cleanup) undo();
  }

  /** Starts showing a new result. Clears everything cached. */
  setColumns(columns: ColumnInfo[]): void {
    this.generation++;
    this.columns = columns;
    this.ctx.font = '600 ' + FONT;
    this.widths = columns.map((c) => {
      const header = Math.ceil(this.ctx.measureText(c.name).width) + PAD * 2 + 4;
      const base = c.kind === 'number' ? 110 : c.kind === 'date' ? 170 : c.kind === 'binary' ? 120 : 200;
      return Math.min(420, Math.max(base, header));
    });
    this.rebuildEdges();
    this.pages.clear();
    this.inflight.clear();
    this.rows = 0;
    this.top = 0;
    this.left = 0;
    this.selected = null;
    this.scroller.scrollLeft = 0;
    this.layout();
  }

  /** The rows behind the grid changed order or number (sort, filter): forget what was cached. */
  resetRows(rows: number): void {
    this.generation++;
    this.pages.clear();
    this.inflight.clear();
    this.rows = rows;
    this.top = 0;
    this.selected = null;
    this.layout();
  }

  /** Which column the rows are sorted by, to mark its header. */
  setSort(sort: { column: number; descending: boolean } | null): void {
    this.sort = sort;
    this.invalidate();
  }

  setOverlay(overlay: GridOverlay): void {
    this.overlay = overlay;
    this.invalidate();
  }

  /** More rows have become available (the query is still streaming) or the count is final. */
  setRowCount(rows: number): void {
    if (rows === this.rows) return;
    this.rows = rows;
    this.layout();
  }

  // ---------- geometry ----------

  private rebuildEdges(): void {
    this.edges = [0];
    for (const w of this.widths) this.edges.push(this.edges[this.edges.length - 1] + w);
  }

  private get contentWidth(): number {
    return this.edges[this.edges.length - 1];
  }

  private maxTop(): number {
    return Math.max(0, this.rows * ROW_H - (this.height - HEADER_H));
  }

  private scrollRange(): number {
    return Math.max(0, this.spacer.offsetHeight - this.scroller.clientHeight);
  }

  private layout(): void {
    this.gutter = Math.max(56, String(this.rows).length * 8 + 22);
    this.spacer.style.height = Math.min(this.rows * ROW_H + HEADER_H, MAX_SCROLL_PX) + 'px';
    this.spacer.style.width = this.gutter + this.contentWidth + 'px';
    this.syncScrollbar();
    this.invalidate();
  }

  private syncScrollbar(): void {
    const max = this.maxTop();
    const target = max <= 0 ? 0 : Math.round((this.top / max) * this.scrollRange());
    this.expectedScrollTop = target;
    this.scroller.scrollTop = target;
  }

  private setTop(top: number): void {
    this.top = Math.min(Math.max(0, top), this.maxTop());
    this.syncScrollbar();
    this.invalidate();
  }

  private columnAt(contentX: number): number {
    for (let c = 0; c < this.widths.length; c++) {
      if (contentX < this.edges[c + 1]) return c;
    }
    return -1;
  }

  private hit(e: MouseEvent): { x: number; y: number; row: number; column: number; resizeColumn: number } {
    const rect = this.scroller.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const contentX = x - this.gutter + this.left;
    const column = x < this.gutter ? -1 : this.columnAt(contentX);
    const row = y < HEADER_H ? -1 : Math.floor((y - HEADER_H + this.top) / ROW_H);
    let resizeColumn = -1;
    if (y < HEADER_H && x >= this.gutter) {
      for (let c = 0; c < this.widths.length; c++) {
        if (Math.abs(contentX - this.edges[c + 1]) <= 4) resizeColumn = c;
      }
    }
    return { x, y, row: row < this.rows ? row : -1, column, resizeColumn };
  }

  private ensureVisible(row: number, column: number): void {
    const bodyHeight = this.height - HEADER_H;
    if (row * ROW_H < this.top) this.top = row * ROW_H;
    else if ((row + 1) * ROW_H > this.top + bodyHeight) this.top = (row + 1) * ROW_H - bodyHeight;
    const bodyWidth = this.width - this.gutter;
    if (this.edges[column] < this.left) this.scroller.scrollLeft = this.edges[column];
    else if (this.edges[column + 1] > this.left + bodyWidth) {
      this.scroller.scrollLeft = Math.min(this.edges[column], this.edges[column + 1] - bodyWidth);
    }
    this.setTop(this.top);
  }

  private select(row: number, column: number): void {
    if (this.rows === 0 || this.columns.length === 0) return;
    row = Math.min(Math.max(0, row), this.rows - 1);
    column = Math.min(Math.max(0, column), this.columns.length - 1);
    this.selected = { row, column };
    this.ensureVisible(row, column);
    this.callbacks.onSelect?.(row, column);
  }

  // ---------- input ----------

  private onScroll(): void {
    // Ignore the echo of our own scrollTop assignments; only a scrollbar
    // drag should move the content through the capped mapping.
    if (Math.abs(this.scroller.scrollTop - this.expectedScrollTop) > 1) {
      const range = this.scrollRange();
      this.top = range <= 0 ? 0 : (this.scroller.scrollTop / range) * this.maxTop();
      this.expectedScrollTop = this.scroller.scrollTop;
    }
    this.left = this.scroller.scrollLeft;
    this.invalidate();
  }

  private onWheel(e: WheelEvent): void {
    // Handled here so one wheel notch always moves the same number of rows,
    // however large the result is.
    e.preventDefault();
    const unit = e.deltaMode === 1 ? ROW_H : e.deltaMode === 2 ? this.height : 1;
    if (e.shiftKey) {
      this.scroller.scrollLeft += e.deltaY * unit;
    } else {
      this.scroller.scrollLeft += e.deltaX * unit;
      this.setTop(this.top + e.deltaY * unit);
    }
  }

  private onMouseDown(e: MouseEvent): void {
    if (e.button !== 0) return;
    const hit = this.hit(e);
    // Clicks on the native scrollbars land outside the painted area.
    if (hit.x > this.width || hit.y > this.height) return;
    if (hit.resizeColumn >= 0) {
      this.drag = { column: hit.resizeColumn, startX: e.clientX, startWidth: this.widths[hit.resizeColumn] };
      e.preventDefault();
      return;
    }
    if (hit.y < HEADER_H && hit.column >= 0) {
      this.callbacks.onSort?.(hit.column);
      return;
    }
    if (hit.row >= 0 && hit.column >= 0) this.select(hit.row, hit.column);
  }

  private onContextMenu(e: MouseEvent): void {
    const hit = this.hit(e);
    if (hit.row < 0 || hit.column < 0) return;
    e.preventDefault();
    this.select(hit.row, hit.column);
    this.callbacks.onContext?.(hit.row, hit.column, e.clientX, e.clientY);
  }

  private onHover(e: MouseEvent): void {
    if (this.drag) return;
    this.scroller.style.cursor = this.hit(e).resizeColumn >= 0 ? 'col-resize' : 'default';
  }

  private onDragMove(e: MouseEvent): void {
    if (!this.drag) return;
    this.widths[this.drag.column] = Math.max(MIN_COL_W, this.drag.startWidth + e.clientX - this.drag.startX);
    this.rebuildEdges();
    // Fitted text depends on the column width, so cached text is stale.
    for (const page of this.pages.values()) page.text = new Array(page.text.length);
    this.layout();
  }

  private onDoubleClick(e: MouseEvent): void {
    const hit = this.hit(e);
    if (hit.row >= 0 && hit.column >= 0) this.callbacks.onActivate?.(hit.row, hit.column);
  }

  private onKeyDown(e: KeyboardEvent): void {
    const current = this.selected ?? { row: Math.floor(this.top / ROW_H), column: 0 };
    const pageRows = Math.max(1, Math.floor((this.height - HEADER_H) / ROW_H) - 1);
    const mod = e.ctrlKey || e.metaKey;
    let { row, column } = current;
    switch (e.key) {
      case 'ArrowDown': row += 1; break;
      case 'ArrowUp': row -= 1; break;
      case 'ArrowRight': column += 1; break;
      case 'ArrowLeft': column -= 1; break;
      case 'PageDown': row += pageRows; break;
      case 'PageUp': row -= pageRows; break;
      case 'Home': if (mod) row = 0; else column = 0; break;
      case 'End': if (mod) row = this.rows - 1; else column = this.columns.length - 1; break;
      case 'Enter':
        if (this.selected) this.callbacks.onActivate?.(row, column);
        e.preventDefault();
        return;
      case 'c':
      case 'C':
        if (mod && this.selected) {
          this.callbacks.onCopy?.(row, column);
          e.preventDefault();
        }
        return;
      default:
        return;
    }
    e.preventDefault();
    this.select(row, column);
  }

  // ---------- page cache ----------

  private decodePage(index: number, u8: Uint8Array): Page {
    const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
    const count = dv.getUint32(0, true);
    const n = count * this.columns.length;
    const starts = new Uint32Array(n);
    const lens = new Int32Array(n);
    let pos = 4;
    for (let i = 0; i < n; i++) {
      const b = u8[pos];
      if (b < 251) {
        starts[i] = pos + 1; lens[i] = b; pos += 1 + b;
      } else if (b === 0xfb) {
        lens[i] = -1; pos += 1;
      } else if (b === 0xfc) {
        const l = dv.getUint16(pos + 1, true);
        starts[i] = pos + 3; lens[i] = l; pos += 3 + l;
      } else if (b === 0xfd) {
        const l = u8[pos + 1] | (u8[pos + 2] << 8) | (u8[pos + 3] << 16);
        starts[i] = pos + 4; lens[i] = l; pos += 4 + l;
      } else {
        const l = dv.getUint32(pos + 1, true);
        starts[i] = pos + 9; lens[i] = l; pos += 9 + l;
      }
    }
    return { first: index * PAGE, count, u8, starts, lens, text: new Array(n), used: ++this.tick };
  }

  private async fetchPage(index: number): Promise<void> {
    if (this.inflight.has(index) || this.columns.length === 0 || this.destroyed) return;
    this.inflight.add(index);
    const generation = this.generation;
    try {
      const u8 = await this.fetchRows(index * PAGE, PAGE);
      if (generation !== this.generation || this.destroyed) return;
      this.pages.set(index, this.decodePage(index, u8));
      if (this.pages.size > MAX_PAGES) {
        let oldest = -1;
        let oldestUsed = Infinity;
        for (const [key, page] of this.pages) {
          if (key !== index && page.used < oldestUsed) {
            oldest = key;
            oldestUsed = page.used;
          }
        }
        this.pages.delete(oldest);
      }
      this.invalidate();
    } catch {
      // The result was closed while the request was in flight.
    } finally {
      if (generation === this.generation) this.inflight.delete(index);
    }
  }

  /** Makes sure the page holding `row` is loaded or being loaded. */
  private want(row: number): Page | undefined {
    const index = Math.floor(row / PAGE);
    const page = this.pages.get(index);
    if (!page || row - page.first >= page.count) {
      // Missing, or fetched while the query was still streaming.
      void this.fetchPage(index);
      return undefined;
    }
    page.used = ++this.tick;
    return page;
  }

  private fit(text: string, available: number): string {
    const width = this.ctx.measureText(text).width;
    if (width <= available) return text;
    let n = Math.max(1, Math.floor((text.length * available) / width) - 1);
    let cut = text.slice(0, n) + '…';
    while (n > 1 && this.ctx.measureText(cut).width > available) {
      n--;
      cut = text.slice(0, n) + '…';
    }
    return cut;
  }

  /** Text to paint for a cell, already cut to the column width. */
  private cell(row: number, column: number): CellText {
    const page = this.want(row);
    if (!page) return undefined;
    const k = (row - page.first) * this.columns.length + column;
    let text = page.text[k];
    if (text === undefined) {
      const len = page.lens[k];
      if (len < 0) {
        text = null;
      } else if (this.columns[column].kind === 'binary') {
        text = `(${formatBytes(len)})`;
      } else {
        const available = this.widths[column] - PAD * 2;
        // No point decoding more than could possibly fit.
        const maxBytes = Math.ceil(available / 3) * 4 + 16;
        const start = page.starts[k];
        const raw = this.decoder.decode(page.u8.subarray(start, start + Math.min(len, maxBytes)));
        text = this.fit(raw.replace(/\s+/g, ' ') + (len > maxBytes ? '…' : ''), available);
      }
      page.text[k] = text;
    }
    return text;
  }

  // ---------- painting ----------

  private readColors() {
    const css = getComputedStyle(this.canvas);
    const get = (name: string) => css.getPropertyValue(name).trim();
    return {
      bg: get('--bg'),
      text: get('--text'),
      muted: get('--muted'),
      zebra: get('--zebra'),
      header: get('--panel'),
      border: get('--border'),
      placeholder: get('--placeholder'),
      accent: get('--accent'),
      selection: get('--selection'),
      changed: get('--changed'),
      deleted: get('--deleted'),
    };
  }

  invalidate(): void {
    if (this.framePending || this.destroyed) return;
    this.framePending = true;
    requestAnimationFrame(() => {
      this.framePending = false;
      if (!this.destroyed) this.draw();
    });
  }

  private draw(): void {
    const { ctx, colors } = this;
    const dpr = window.devicePixelRatio || 1;
    const w = this.scroller.clientWidth;
    const h = this.scroller.clientHeight;
    if (w === 0 || h === 0) return; // hidden tab
    if (this.canvas.width !== Math.round(w * dpr) || this.canvas.height !== Math.round(h * dpr)) {
      this.canvas.width = Math.round(w * dpr);
      this.canvas.height = Math.round(h * dpr);
      this.canvas.style.width = w + 'px';
      this.canvas.style.height = h + 'px';
    }
    this.width = w;
    this.height = h;
    if (this.top > this.maxTop()) this.top = this.maxTop();
    this.left = this.scroller.scrollLeft;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = colors.bg;
    ctx.fillRect(0, 0, w, h);
    ctx.font = FONT;
    ctx.textBaseline = 'middle';

    const ncol = this.columns.length;
    if (ncol === 0) return;
    const gutter = this.gutter;
    const firstRow = Math.floor(this.top / ROW_H);
    const y0 = HEADER_H - (this.top - firstRow * ROW_H);
    const lastRow = Math.min(this.rows, firstRow + Math.ceil((h - y0) / ROW_H));
    const firstCol = Math.max(0, this.columnAt(this.left));
    let lastCol = firstCol;
    while (lastCol < ncol && this.edges[lastCol] - this.left < w - gutter) lastCol++;
    const xOf = (c: number) => gutter + this.edges[c] - this.left;

    // Cells.
    ctx.save();
    ctx.beginPath();
    ctx.rect(gutter, HEADER_H, w - gutter, h - HEADER_H);
    ctx.clip();
    for (let r = firstRow; r < lastRow; r++) {
      const y = y0 + (r - firstRow) * ROW_H;
      const selectedRow = this.selected?.row === r;
      const deleted = this.overlay.deletedRows.has(r);
      if (deleted || selectedRow || r % 2 === 1) {
        ctx.fillStyle = deleted ? colors.deleted : selectedRow ? colors.selection : colors.zebra;
        ctx.fillRect(gutter, y, w - gutter, ROW_H);
      }
      const mid = y + ROW_H / 2;
      for (let c = firstCol; c < lastCol; c++) {
        const x = xOf(c);
        const width = this.widths[c];
        const key = `${r}:${c}`;
        if (this.overlay.cells.has(key)) {
          // A change waiting to be applied: shown in place of the stored value.
          const pending = this.overlay.cells.get(key);
          ctx.fillStyle = colors.changed;
          ctx.fillRect(x, y, width - 1, ROW_H);
          ctx.fillStyle = colors.accent;
          ctx.textAlign = 'left';
          ctx.fillText(pending == null ? 'NULL' : this.fit(pending.replace(/\s+/g, ' '), width - PAD * 2), x + PAD, mid);
          continue;
        }
        const text = this.cell(r, c);
        if (text === undefined) {
          ctx.fillStyle = colors.placeholder;
          ctx.fillRect(x + PAD, mid - 3, Math.max(8, width * 0.6 - PAD * 2), 6);
        } else if (text === null) {
          ctx.fillStyle = colors.muted;
          ctx.textAlign = 'left';
          ctx.fillText('NULL', x + PAD, mid);
        } else {
          const kind = this.columns[c].kind;
          ctx.fillStyle = kind === 'binary' ? colors.muted : colors.text;
          if (kind === 'number') {
            ctx.textAlign = 'right';
            ctx.fillText(text, x + width - PAD, mid);
          } else {
            ctx.textAlign = 'left';
            ctx.fillText(text, x + PAD, mid);
          }
        }
      }
    }
    ctx.fillStyle = colors.border;
    for (let c = firstCol; c < lastCol; c++) {
      ctx.fillRect(xOf(c + 1) - 1, HEADER_H, 1, Math.min(h, y0 + (lastRow - firstRow) * ROW_H) - HEADER_H);
    }
    if (this.selected && this.selected.row >= firstRow && this.selected.row < lastRow) {
      const { row, column } = this.selected;
      ctx.strokeStyle = colors.accent;
      ctx.lineWidth = 2;
      ctx.strokeRect(xOf(column) + 0.5, y0 + (row - firstRow) * ROW_H + 1, this.widths[column] - 2, ROW_H - 2);
    }
    ctx.restore();

    // Row numbers.
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, HEADER_H, gutter, h - HEADER_H);
    ctx.clip();
    ctx.fillStyle = colors.header;
    ctx.fillRect(0, HEADER_H, gutter, h - HEADER_H);
    ctx.fillStyle = colors.muted;
    ctx.textAlign = 'right';
    for (let r = firstRow; r < lastRow; r++) {
      ctx.fillText(String(r + 1), gutter - 10, y0 + (r - firstRow) * ROW_H + ROW_H / 2);
    }
    ctx.restore();

    // Column headers.
    ctx.fillStyle = colors.header;
    ctx.fillRect(0, 0, w, HEADER_H);
    ctx.save();
    ctx.beginPath();
    ctx.rect(gutter, 0, w - gutter, HEADER_H);
    ctx.clip();
    ctx.font = '600 ' + FONT;
    for (let c = firstCol; c < lastCol; c++) {
      const x = xOf(c);
      const width = this.widths[c];
      const arrow = this.sort?.column === c ? (this.sort.descending ? ' \u25BE' : ' \u25B4') : '';
      const name = this.fit(this.columns[c].name, width - PAD * 2 - (arrow ? 14 : 0)) + arrow;
      ctx.fillStyle = colors.text;
      if (this.columns[c].kind === 'number') {
        ctx.textAlign = 'right';
        ctx.fillText(name, x + width - PAD, HEADER_H / 2);
      } else {
        ctx.textAlign = 'left';
        ctx.fillText(name, x + PAD, HEADER_H / 2);
      }
      ctx.fillStyle = colors.border;
      ctx.fillRect(x + width - 1, 0, 1, HEADER_H);
    }
    ctx.restore();
    ctx.fillStyle = colors.border;
    ctx.fillRect(0, HEADER_H - 1, w, 1);
    ctx.fillRect(gutter - 1, 0, 1, h);

    // Fetch one page ahead in each direction so steady scrolling never
    // reaches rows that are not loaded.
    if (firstRow - PAGE / 2 >= 0) this.want(firstRow - PAGE / 2);
    if (lastRow + PAGE / 2 < this.rows) this.want(lastRow + PAGE / 2);
  }
}
