// Application state. Components read and call into these classes; nothing
// else talks to the backend about connections, tabs or running statements.

import { backend } from './backend';
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
}

export interface MessageLine {
  id: number;
  sql: string;
  state: 'running' | 'ok' | 'error';
  text: string;
  elapsedMs: number | null;
}

let tabCounter = 0;
let lineCounter = 0;

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
  /** A result id, or 'messages'. */
  activeResult = $state<number | 'messages'>('messages');
  lastRunMs = $state<number | null>(null);

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
  maxRows = $state<number | null>(10_000);
  showNavigator = $state(true);
  showOutput = $state(true);
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
    this.addTab();
    if (info.database) void this.loadCatalog(info.database);
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
          l.text = `${formatCount(m.rows)} ${m.rows === 1 ? 'row' : 'rows'}` + (m.truncated ? ' (row limit reached, more exist)' : '');
        }
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
        break;
      }
      case 'failed': {
        const l = lines.get(m.index);
        if (l) Object.assign(l, { state: 'error', text: m.message, elapsedMs: m.elapsedMs });
        tab.activeResult = 'messages';
        break;
      }
      case 'database':
        this.database = m.name;
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

  get activeWorkspace(): WorkspaceState | null {
    return this.workspaces.find((w) => w.info.id === this.active) ?? null;
  }

  async load(): Promise<void> {
    this.info = await backend.appInfo();
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
