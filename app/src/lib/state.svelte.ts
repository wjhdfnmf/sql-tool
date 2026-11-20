// Application state. Components read and call into these classes; nothing
// else talks to the backend about connections, tabs or running statements.

import { backend } from './backend';
import type { GraphState } from './graph.svelte';
import { settings } from './settings.svelte';
import {
  asUiError,
  formatCount,
  type AppInfo,
  type ColumnInfo,
  type Concern,
  type Profile,
  type RunMode,
  type RunMsg,
  type TreeTarget,
  type WorkspaceInfo,
} from './types';

export interface ResultTab {
  id: number;
  title: string;
  columns: ColumnInfo[];
  rows: number;
  done: boolean;
  truncated: boolean;
  elapsedMs: number | null;
  /** The statement that produced this result, to run it again after edits. */
  sql: string;
  /** Changes made in the grid and not yet applied or discarded. Kept here so that closing the window can ask first. */
  pendingEdits?: number;
}

/** What a designer tab is editing. A null name means something new is being made. */
export type TabDesign = { kind: 'table'; schema: string; table: string | null } | { kind: 'schema'; schema: string | null };

export interface MessageLine {
  id: number;
  sql: string;
  state: 'running' | 'ok' | 'error';
  text: string;
  elapsedMs: number | null;
}

let tabCounter = 0;
let lineCounter = 0;
/** Message lines kept per tab. A script of many thousands of statements would otherwise fill the page. */
const MAX_MESSAGES = 1000;

// Open query tabs are remembered per saved connection, so closing the app
// does not lose a query in progress. They are kept in the webview's local
// storage, on this computer only and not encrypted, like the history. A tab
// holding a statement with a password in it is never written there.
const TABS_KEY = 'sqltool.tabs.';
const MAX_SAVED_TABS = 30;
const MAX_SAVED_TAB_CHARS = 200_000;
const MAX_SAVED_CHARS = 1_000_000;
const HOLDS_PASSWORD =
  /identified\s+(by|with|via)|set\s+password|password\s*\(|master_password|source_password|authentication_string/i;

interface SavedTab {
  title: string;
  text: string;
  /** The file the tab was opened from or saved to, so Save writes back to it after a restart. */
  filePath?: string;
  /** The text differs from what is in the file. */
  dirty?: boolean;
}

function loadSavedTabs(profileId: string | null): SavedTab[] {
  if (!profileId) return [];
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(TABS_KEY + profileId) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((t): t is SavedTab => typeof t?.title === 'string' && typeof t?.text === 'string')
      .slice(0, MAX_SAVED_TABS);
  } catch {
    // Unreadable or unavailable storage: start with an empty tab.
    return [];
  }
}

// The default schema chosen for a connection is kept in the connection's
// own settings (by the backend), so there is one record of it. Versions
// 0.1.4 and 0.1.5 kept it here instead; that copy is read once, moved into
// the settings, and removed.
const SCHEMA_KEY = 'sqltool.schema.';

function takeOldRememberedSchema(profileId: string | null): string | null {
  if (!profileId) return null;
  try {
    const schema = localStorage.getItem(SCHEMA_KEY + profileId);
    localStorage.removeItem(SCHEMA_KEY + profileId);
    return schema;
  } catch {
    return null;
  }
}

/** Forgets what was kept for a connection that has been deleted: its query tabs and default schema. */
export function forgetSavedTabs(profileId: string): void {
  try {
    localStorage.removeItem(TABS_KEY + profileId);
    localStorage.removeItem(SCHEMA_KEY + profileId);
  } catch {
    // Nothing was stored.
  }
}

export class EditorTab {
  readonly id = crypto.randomUUID();
  title = $state('');
  text = $state('');
  /** The file this tab was opened from or saved to, if any. */
  filePath = $state<string | null>(null);
  /** The text as last opened or saved, to tell whether there are unsaved changes. */
  savedText = $state('');
  running = $state(false);
  results = $state<ResultTab[]>([]);
  messages = $state<MessageLine[]>([]);
  /** Earlier message lines dropped to keep the list short. */
  hiddenMessages = $state(0);
  /** A result id, or 'messages'. */
  activeResult = $state<number | 'messages'>('messages');
  lastRunMs = $state<number | null>(null);
  /** Set when this tab is a table or schema designer instead of a query. */
  design = $state<TabDesign | null>(null);
  /** The designer has changes that have not been applied. */
  designDirty = $state(false);
  /** How far a run in time slices has got, while one is running. */
  slices = $state<{ done: number; total: number; rows: number; etaMs: number | null; threads: number } | null>(null);
  /** The range and slice length last used to run this tab in time slices. */
  sliceSetup: { start: string; end: string; every: number; unit: 'minutes' | 'hours' | 'days' } | null = null;
  /** The graph set up from one of this tab's results, kept so that leaving it and coming back finds it as it was. */
  graph = $state<GraphState | null>(null);

  constructor(title: string, text = '') {
    this.title = title;
    this.text = text;
    this.savedText = text;
  }

  get dirty(): boolean {
    return this.text !== this.savedText;
  }

  report(text: string, state: MessageLine['state'] = 'error'): void {
    this.messages.push({ id: ++lineCounter, sql: '', state, text, elapsedMs: null });
    if (state === 'error') this.activeResult = 'messages';
  }
}

export class WorkspaceState {
  readonly info: WorkspaceInfo;
  tabs = $state<EditorTab[]>([]);
  activeTabId = $state('');
  database = $state<string | null>(null);
  /** Rows kept per result set; null means no limit. */
  maxRows = $state<number | null>(settings.current.defaultRowLimit);
  showNavigator = $state(true);
  showOutput = $state(true);
  /** The graph builder is in front, in place of the query, for tabs that have a graph. */
  graphOpen = $state(false);
  /** The place in the bottom row where the result in front shows its row count, state and notices. */
  statusHost = $state<HTMLElement | undefined>();
  /** What is selected in the schema tree, shown in the information pane. */
  selected = $state<TreeTarget | null>(null);
  /** Bumped to make the schema tree reload. */
  treeVersion = $state(0);
  /** Table and column names per schema, for editor completion. */
  catalog = $state<Record<string, Record<string, string[]>>>({});
  /**
   * Set by the workspace screen: asks the user to confirm risky statements
   * on a production connection. Resolves to true to go ahead.
   */
  confirmRun: ((concerns: Concern[]) => Promise<boolean>) | null = null;

  constructor(info: WorkspaceInfo) {
    this.info = info;
    this.database = info.database;
    const saved = settings.current.rememberTabs ? loadSavedTabs(info.profileId) : [];
    for (const tab of saved) {
      const restored = this.addTab(tab.text, tab.title);
      if (typeof tab.filePath === 'string' && tab.filePath !== '') {
        restored.filePath = tab.filePath;
        // Edits made before the app closed and never saved are still unsaved.
        if (tab.dirty) restored.savedText = `${tab.text}\u0000unsaved`;
      }
      // Keep new tabs from reusing the number of a restored "Query 3".
      const numbered = /^Query (\d+)$/.exec(tab.title);
      if (numbered) tabCounter = Math.max(tabCounter, Number(numbered[1]));
    }
    if (saved.length === 0) this.addTab();
    else this.activeTabId = this.tabs[0].id;
    if (info.database) void this.loadCatalog(info.database);
    // A default schema remembered by an earlier version, if the connection has none of its own.
    const old = takeOldRememberedSchema(info.profileId);
    if (old && !info.database) {
      // It may have been dropped since; then there is simply no default.
      this.setDefaultSchema(old).catch(() => {});
    }
  }

  /**
   * Makes a schema the default for every tab of this connection. Each tab
   * has a session of its own; the backend selects the schema in each before
   * its next run, and in tabs opened later.
   */
  async setDefaultSchema(schema: string): Promise<void> {
    await backend.setDefaultSchema(this.info.id, schema);
    this.database = schema;
    void this.loadCatalog(schema);
    // The backend has written it into the connection's settings; show that on the Connections page too.
    void app.refreshProfiles().catch(() => {});
  }

  /** Writes the open tabs' text to local storage, for the next time this connection is opened. */
  saveTabs(): void {
    const profileId = this.info.profileId;
    if (!profileId) return;
    if (!settings.current.rememberTabs) {
      forgetSavedTabs(profileId);
      return;
    }
    const saved: SavedTab[] = [];
    let total = 0;
    for (const tab of this.tabs) {
      const text = tab.text;
      // Designers are not remembered: they are reopened from the table or schema.
      if (tab.design) continue;
      if (text.trim() === '' || text.length > MAX_SAVED_TAB_CHARS || HOLDS_PASSWORD.test(text)) continue;
      total += text.length;
      if (total > MAX_SAVED_CHARS || saved.length >= MAX_SAVED_TABS) break;
      saved.push(tab.filePath ? { title: tab.title, text, filePath: tab.filePath, dirty: tab.dirty } : { title: tab.title, text });
    }
    try {
      if (saved.length === 0) localStorage.removeItem(TABS_KEY + profileId);
      else localStorage.setItem(TABS_KEY + profileId, JSON.stringify(saved));
    } catch {
      // Storage is full or unavailable. Remembering tabs is a convenience.
    }
  }

  /** Loads a schema's table and column names for completion, once. */
  async loadCatalog(schema: string, force = false): Promise<void> {
    if (!force && this.catalog[schema]) return;
    try {
      const tables: Record<string, string[]> = {};
      for (const [table, column] of await backend.completionColumns(this.info.id, schema)) {
        (tables[table] ??= []).push(column);
      }
      this.catalog[schema] = tables;
    } catch {
      // Completion is a convenience; a schema that cannot be read is skipped.
    }
  }

  get activeTab(): EditorTab {
    return this.tabs.find((t) => t.id === this.activeTabId) ?? this.tabs[0];
  }

  addTab(text = '', title?: string): EditorTab {
    const tab = new EditorTab(title ?? `Query ${++tabCounter}`, text);
    this.tabs.push(tab);
    this.activeTabId = tab.id;
    return tab;
  }

  /** Opens a designer tab, or goes to the one already open for the same table or schema. */
  openDesign(design: TabDesign): EditorTab {
    const existing = this.tabs.find((t) => {
      const open = t.design;
      if (!open) return false;
      if (open.kind === 'schema' && design.kind === 'schema') return open.schema !== null && open.schema === design.schema;
      if (open.kind === 'table' && design.kind === 'table') {
        return open.table !== null && open.table === design.table && open.schema === design.schema;
      }
      return false;
    });
    if (existing) {
      this.activeTabId = existing.id;
      return existing;
    }
    const title =
      design.kind === 'table'
        ? design.table === null ? 'New table' : `${design.table} (table)`
        : design.schema === null ? 'New schema' : `${design.schema} (schema)`;
    const tab = new EditorTab(title);
    tab.design = design;
    this.tabs.push(tab);
    this.activeTabId = tab.id;
    return tab;
  }

  closeTab(tab: EditorTab): void {
    if (tab.running) void backend.cancel(this.info.id, tab.id);
    this.releaseResults(tab);
    void backend.closeTab(this.info.id, tab.id);
    const index = this.tabs.findIndex((t) => t.id === tab.id);
    this.tabs.splice(index, 1);
    if (this.tabs.length === 0) this.addTab();
    else if (this.activeTabId === tab.id) this.activeTabId = this.tabs[Math.max(0, index - 1)].id;
  }

  private releaseResults(tab: EditorTab): void {
    const ids = tab.results.map((r) => r.id);
    tab.results = [];
    if (ids.length > 0) void backend.closeResults(ids);
  }

  /**
   * Runs statements in the tab's own session. `keep` leaves the tab's
   * current results and messages in place and only adds to the messages,
   * for small actions such as changing the default schema.
   */
  async run(tab: EditorTab, script: string, mode: RunMode, keep = false): Promise<void> {
    if (tab.running || script.trim() === '') return;
    if (this.info.production && this.confirmRun) {
      try {
        const concerns = await backend.reviewScript(script, mode);
        if (concerns.length > 0 && !(await this.confirmRun(concerns))) {
          tab.report('Cancelled. Nothing was run.', 'ok');
          return;
        }
      } catch (e) {
        tab.report(asUiError(e).message);
        return;
      }
    }
    if (!keep) {
      this.releaseResults(tab);
      tab.messages = [];
      tab.hiddenMessages = 0;
      tab.activeResult = 'messages';
    }
    tab.lastRunMs = null;
    tab.running = true;
    const lines = new Map<number, MessageLine>();
    try {
      await backend.run(this.info.id, tab.id, script, mode, this.maxRows, (message) => this.onRunMsg(tab, lines, message));
    } catch (e) {
      tab.report(asUiError(e).message);
    } finally {
      tab.running = false;
    }
  }

  /**
   * Runs one query once for each slice of a time range, on connections of
   * its own, and shows the rows of all the slices as one result. The query
   * marks its range with {start} and {end}.
   */
  async runSliced(tab: EditorTab, sql: string, start: string, end: string, stepSecs: number): Promise<void> {
    if (tab.running) return;
    this.releaseResults(tab);
    tab.messages = [];
    tab.hiddenMessages = 0;
    tab.activeResult = 'messages';
    tab.lastRunMs = null;
    tab.running = true;
    tab.slices = null;
    const lines = new Map<number, MessageLine>();
    try {
      await backend.runSliced(this.info.id, tab.id, sql, start, end, stepSecs, this.maxRows, (message) => this.onRunMsg(tab, lines, message));
    } catch (e) {
      tab.report(asUiError(e).message);
    } finally {
      tab.running = false;
      tab.slices = null;
    }
  }

  /**
   * Runs statements on a separate connection, without touching any editor
   * tab. For actions such as creating a schema. Returns the error, if any.
   */
  async execute(script: string): Promise<string | null> {
    let error: string | null = null;
    try {
      await backend.run(this.info.id, `actions-${this.info.id}`, script, { kind: 'all' }, 1000, (message) => {
        if (message.type === 'failed') error = message.message;
        if (message.type === 'resultSet') void backend.closeResults([message.resultId]);
      });
    } catch (e) {
      error = asUiError(e).message;
    }
    return error;
  }

  async cancel(tab: EditorTab): Promise<void> {
    if (!tab.running) return;
    try {
      await backend.cancel(this.info.id, tab.id);
    } catch (e) {
      tab.report('Could not cancel: ' + asUiError(e).message);
    }
  }

  private onRunMsg(tab: EditorTab, lines: Map<number, MessageLine>, m: RunMsg): void {
    const result = (id: number) => tab.results.find((r) => r.id === id);
    switch (m.type) {
      case 'started': {
        if (tab.messages.length >= MAX_MESSAGES) {
          // Drop the oldest in one go, not one per statement.
          const cut = tab.messages.length - MAX_MESSAGES + 200;
          tab.messages.splice(0, cut);
          tab.hiddenMessages += cut;
        }
        tab.messages.push({ id: ++lineCounter, sql: m.sql, state: 'running', text: 'Running…', elapsedMs: null });
        // Read the line back so later updates go through the reactive copy.
        lines.set(m.index, tab.messages[tab.messages.length - 1]);
        break;
      }
      case 'resultSet':
        tab.results.push({
          id: m.resultId,
          title: `Result ${tab.results.length + 1}`,
          columns: m.columns,
          rows: 0,
          done: false,
          truncated: false,
          elapsedMs: null,
          sql: m.sql,
        });
        if (tab.activeResult === 'messages') tab.activeResult = m.resultId;
        break;
      case 'progress': {
        const r = result(m.resultId);
        if (r) r.rows = m.rows;
        break;
      }
      case 'resultDone': {
        const r = result(m.resultId);
        if (r) Object.assign(r, { rows: m.rows, done: true, truncated: m.truncated, elapsedMs: m.elapsedMs });
        const l = lines.get(m.index);
        if (l) {
          l.state = 'ok';
          l.elapsedMs = m.elapsedMs;
          l.text =
            `${formatCount(m.rows)} ${m.rows === 1 ? 'row' : 'rows'}` +
            (tab.slices ? `, from ${formatCount(tab.slices.done)} time ${tab.slices.done === 1 ? 'slice' : 'slices'}` : '') +
            (m.truncated ? (tab.slices ? ' (stopped before the end of the range)' : ' (row limit reached, more exist)') : '');
        }
        lines.delete(m.index);
        break;
      }
      case 'done': {
        const l = lines.get(m.index);
        if (l) {
          l.state = 'ok';
          l.elapsedMs = m.elapsedMs;
          l.text =
            `${formatCount(m.affectedRows)} ${m.affectedRows === 1 ? 'row' : 'rows'} affected` +
            (m.warnings > 0 ? `, ${m.warnings} ${m.warnings === 1 ? 'warning' : 'warnings'}` : '') +
            (m.info ? `. ${m.info}` : '');
        }
        lines.delete(m.index);
        break;
      }
      case 'failed': {
        const l = lines.get(m.index);
        if (l) Object.assign(l, { state: 'error', text: m.message, elapsedMs: m.elapsedMs });
        tab.activeResult = 'messages';
        break;
      }
      case 'slices': {
        tab.slices = { done: m.done, total: m.total, rows: m.rows, etaMs: m.etaMs, threads: m.threads };
        const l = lines.get(0);
        if (l && l.state === 'running') l.text = `Running in time slices: ${formatCount(m.done)} of ${formatCount(m.total)} done, ${formatCount(m.rows)} rows so far`;
        break;
      }
      case 'reconnected':
        tab.messages.push({
          id: ++lineCounter,
          sql: '',
          state: 'ok',
          text: 'The server had closed this tab\'s connection, most likely because it sat idle. A new one was opened. Session variables, temporary tables and any open transaction did not carry over.',
          elapsedMs: null,
        });
        break;
      case 'database':
        this.database = m.name;
        // USE in a script changes the connection's default; its settings were updated to match.
        void app.refreshProfiles().catch(() => {});
        if (m.name) void this.loadCatalog(m.name);
        break;
      case 'finished':
        tab.lastRunMs = m.elapsedMs;
        if (m.total === 0) tab.report('Nothing to run: no statement found.');
        break;
    }
  }
}

class AppState {
  info = $state<AppInfo | null>(null);
  profiles = $state<Profile[]>([]);
  workspaces = $state<WorkspaceState[]>([]);
  /** 'home' or a workspace id. */
  active = $state<'home' | number>('home');
  /** The Settings screen is open. */
  showSettings = $state(false);
  /** The help screen (shortcuts and where things are) is open. */
  showHelp = $state(false);
  /** The list of MySQL Workbench's connections, to bring some over, is open. */
  showImport = $state(false);
  /** The end of the row of connection tabs, where the connection in front shows its buttons. */
  toolsHost = $state<HTMLElement | undefined>();

  get activeWorkspace(): WorkspaceState | null {
    return this.workspaces.find((w) => w.info.id === this.active) ?? null;
  }

  async load(): Promise<void> {
    this.info = await backend.appInfo();
    // Before the connections: a damaged connections file must not keep the
    // theme and the other settings from loading.
    await settings.load().catch(() => {});
    this.profiles = await backend.profilesList();
  }

  async refreshProfiles(): Promise<void> {
    this.profiles = await backend.profilesList();
  }

  open(info: WorkspaceInfo): WorkspaceState {
    const workspace = new WorkspaceState(info);
    this.workspaces.push(workspace);
    this.active = info.id;
    return workspace;
  }

  async close(workspace: WorkspaceState): Promise<void> {
    workspace.saveTabs();
    for (const tab of workspace.tabs) {
      const ids = tab.results.map((r) => r.id);
      if (ids.length > 0) void backend.closeResults(ids);
    }
    const index = this.workspaces.findIndex((w) => w.info.id === workspace.info.id);
    this.workspaces.splice(index, 1);
    if (this.active === workspace.info.id) this.active = this.workspaces[index - 1]?.info.id ?? this.workspaces[0]?.info.id ?? 'home';
    await backend.disconnect(workspace.info.id);
  }
}

export const app = new AppState();
