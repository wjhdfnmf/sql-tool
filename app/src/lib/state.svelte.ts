// Application state. Components read and call into these classes; nothing
// else talks to the backend about connections, tabs or running statements.

import { backend } from './backend';
import {
  asUiError,
  formatCount,
  type AppInfo,
  type ColumnInfo,
  type Profile,
  type RunMode,
  type RunMsg,
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
}

export interface MessageLine {
  index: number;
  sql: string;
  state: 'running' | 'ok' | 'error';
  text: string;
  elapsedMs: number | null;
}

let tabCounter = 0;

export class EditorTab {
  readonly id = crypto.randomUUID();
  title = $state('');
  text = $state('');
  running = $state(false);
  results = $state<ResultTab[]>([]);
  messages = $state<MessageLine[]>([]);
  /** A result id, or 'messages'. */
  activeResult = $state<number | 'messages'>('messages');
  lastRunMs = $state<number | null>(null);

  constructor(title: string, text = '') {
    this.title = title;
    this.text = text;
  }
}

export class WorkspaceState {
  readonly info: WorkspaceInfo;
  tabs = $state<EditorTab[]>([]);
  activeTabId = $state('');
  database = $state<string | null>(null);
  /** Rows kept per result set; null means no limit. */
  maxRows = $state<number | null>(10_000);

  constructor(info: WorkspaceInfo) {
    this.info = info;
    this.database = info.database;
    this.addTab();
  }

  get activeTab(): EditorTab {
    return this.tabs.find((t) => t.id === this.activeTabId) ?? this.tabs[0];
  }

  addTab(text = ''): EditorTab {
    const tab = new EditorTab(`Query ${++tabCounter}`, text);
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

  async run(tab: EditorTab, script: string, mode: RunMode): Promise<void> {
    if (tab.running || script.trim() === '') return;
    this.releaseResults(tab);
    tab.messages = [];
    tab.activeResult = 'messages';
    tab.lastRunMs = null;
    tab.running = true;
    try {
      await backend.run(this.info.id, tab.id, script, mode, this.maxRows, (message) => this.onRunMsg(tab, message));
    } catch (e) {
      tab.messages.push({ index: tab.messages.length, sql: '', state: 'error', text: asUiError(e).message, elapsedMs: null });
      tab.activeResult = 'messages';
    } finally {
      tab.running = false;
    }
  }

  async cancel(tab: EditorTab): Promise<void> {
    if (!tab.running) return;
    try {
      await backend.cancel(this.info.id, tab.id);
    } catch (e) {
      tab.messages.push({ index: tab.messages.length, sql: '', state: 'error', text: 'Could not cancel: ' + asUiError(e).message, elapsedMs: null });
    }
  }

  private onRunMsg(tab: EditorTab, m: RunMsg): void {
    const line = (index: number) => tab.messages.find((l) => l.index === index);
    const result = (id: number) => tab.results.find((r) => r.id === id);
    switch (m.type) {
      case 'started':
        tab.messages.push({ index: m.index, sql: m.sql, state: 'running', text: 'Running…', elapsedMs: null });
        break;
      case 'resultSet':
        tab.results.push({
          id: m.resultId,
          title: `Result ${tab.results.length + 1}`,
          columns: m.columns,
          rows: 0,
          done: false,
          truncated: false,
          elapsedMs: null,
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
        const l = line(m.index);
        if (l) {
          l.state = 'ok';
          l.elapsedMs = m.elapsedMs;
          l.text = `${formatCount(m.rows)} ${m.rows === 1 ? 'row' : 'rows'}` + (m.truncated ? ' (row limit reached, more exist)' : '');
        }
        break;
      }
      case 'done': {
        const l = line(m.index);
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
        const l = line(m.index);
        if (l) Object.assign(l, { state: 'error', text: m.message, elapsedMs: m.elapsedMs });
        tab.activeResult = 'messages';
        break;
      }
      case 'database':
        this.database = m.name;
        break;
      case 'finished':
        tab.lastRunMs = m.elapsedMs;
        if (m.total === 0) {
          tab.messages.push({ index: 0, sql: '', state: 'error', text: 'Nothing to run: no statement found.', elapsedMs: null });
        }
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
