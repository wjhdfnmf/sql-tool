// The single doorway from the UI to the Rust side. Components never call
// Tauri directly, which is what lets the UI also run in a plain browser
// against the mock backend during development.

import { Channel, invoke } from '@tauri-apps/api/core';
import { mockBackend } from './mock';
import type {
  AppInfo,
  CellValue,
  ColumnEntry,
  HistoryEntry,
  Profile,
  RunMode,
  RunMsg,
  SchemaObjects,
  SelfTestConfig,
  ServerInfo,
  WorkspaceInfo,
} from './types';

export interface Backend {
  appInfo(): Promise<AppInfo>;
  profilesList(): Promise<Profile[]>;
  /** `password` null leaves a stored password as it is. */
  profileSave(profile: Profile, password: string | null): Promise<Profile>;
  profileDelete(id: string): Promise<void>;
  connectionTest(profile: Profile, password: string | null): Promise<ServerInfo>;
  /** `password` null uses the keychain; rejects with kind `password-required` if there is none. */
  connect(profileId: string, password: string | null): Promise<WorkspaceInfo>;
  connectDemo(): Promise<WorkspaceInfo>;
  disconnect(workspaceId: number): Promise<void>;
  closeTab(workspaceId: number, tabId: string): Promise<void>;
  /** Resolves when every statement has finished; progress arrives through `onEvent`. */
  run(
    workspaceId: number,
    tabId: string,
    script: string,
    mode: RunMode,
    maxRows: number | null,
    onEvent: (message: RunMsg) => void,
  ): Promise<void>;
  cancel(workspaceId: number, tabId: string): Promise<void>;
  /** Rows as raw bytes: u32 row count, then length-prefixed cells. */
  getRows(resultId: number, offset: number, count: number): Promise<Uint8Array>;
  getCell(resultId: number, row: number, column: number): Promise<CellValue>;
  closeResults(resultIds: number[]): Promise<void>;
  schemaList(workspaceId: number): Promise<string[]>;
  schemaObjects(workspaceId: number, schema: string): Promise<SchemaObjects>;
  tableColumns(workspaceId: number, schema: string, table: string): Promise<ColumnEntry[]>;
  historyList(search: string | null, limit: number): Promise<HistoryEntry[]>;
  historyClear(): Promise<void>;
  selftestConfig(): Promise<SelfTestConfig | null>;
  selftestReport(report: string): Promise<void>;
}

const tauriBackend: Backend = {
  appInfo: () => invoke('app_info'),
  profilesList: () => invoke('profiles_list'),
  profileSave: (profile, password) => invoke('profile_save', { profile, password }),
  profileDelete: (id) => invoke('profile_delete', { id }),
  connectionTest: (profile, password) => invoke('connection_test', { profile, password }),
  connect: (profileId, password) => invoke('connect', { profileId, password }),
  connectDemo: () => invoke('connect_demo'),
  disconnect: (workspaceId) => invoke('disconnect', { workspaceId }),
  closeTab: (workspaceId, tabId) => invoke('close_tab', { workspaceId, tabId }),
  run: (workspaceId, tabId, script, mode, maxRows, onEvent) => {
    const channel = new Channel<RunMsg>();
    channel.onmessage = onEvent;
    return invoke('run', { workspaceId, tabId, script, mode, maxRows, onEvent: channel });
  },
  cancel: (workspaceId, tabId) => invoke('cancel', { workspaceId, tabId }),
  getRows: async (resultId, offset, count) => {
    const data = await invoke<ArrayBuffer | number[]>('get_rows', { resultId, offset, count });
    return data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
  },
  getCell: (resultId, row, column) => invoke('get_cell', { resultId, row, column }),
  closeResults: (resultIds) => invoke('close_results', { resultIds }),
  schemaList: (workspaceId) => invoke('schema_list', { workspaceId }),
  schemaObjects: (workspaceId, schema) => invoke('schema_objects', { workspaceId, schema }),
  tableColumns: (workspaceId, schema, table) => invoke('table_columns', { workspaceId, schema, table }),
  historyList: (search, limit) => invoke('history_list', { search, limit }),
  historyClear: () => invoke('history_clear'),
  selftestConfig: () => invoke('selftest_config'),
  selftestReport: (report) => invoke('selftest_report', { report }),
};

/** True inside the desktop app, false in a plain browser. */
export const inDesktopApp = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const backend: Backend = inDesktopApp ? tauriBackend : mockBackend;
