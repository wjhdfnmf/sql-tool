// The single doorway from the UI to the Rust side. Components never call
// Tauri directly, which is what lets the UI also run in a plain browser
// against the mock backend during development.

import { Channel, invoke } from '@tauri-apps/api/core';
import { mockBackend } from './mock';
import type {
  AppInfo,
  CellValue,
  Charset,
  ColumnEntry,
  HistoryEntry,
  ObjectKind,
  Profile,
  RunMode,
  RunMsg,
  SchemaDefaults,
  SchemaObjects,
  ScriptFile,
  SelfTestConfig,
  ServerInfo,
  TableDetails,
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
  tableDetails(workspaceId: number, schema: string, table: string): Promise<TableDetails | null>;
  schemaDefaults(workspaceId: number, schema: string): Promise<SchemaDefaults | null>;
  charsets(workspaceId: number): Promise<Charset[]>;
  collations(workspaceId: number, charset: string): Promise<string[]>;
  /** SQL to create or alter a schema, for review. Rejects if a name is not acceptable. */
  schemaDdl(alter: boolean, name: string, charset: string | null, collation: string | null): Promise<string>;
  /** A script that changes an existing object, built from its current definition. */
  alterScript(workspaceId: number, kind: ObjectKind, schema: string, name: string): Promise<string>;
  createTemplate(kind: ObjectKind, schema: string | null): Promise<string>;
  /** Shows the native open dialog. Null if the user cancels. */
  scriptOpen(): Promise<ScriptFile | null>;
  /** Saves to `path`, or asks where if `path` is null. Null if the user cancels. */
  scriptSave(path: string | null, suggestedName: string, text: string): Promise<ScriptFile | null>;
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
  tableDetails: (workspaceId, schema, table) => invoke('table_details', { workspaceId, schema, table }),
  schemaDefaults: (workspaceId, schema) => invoke('schema_defaults', { workspaceId, schema }),
  charsets: (workspaceId) => invoke('charsets', { workspaceId }),
  collations: (workspaceId, charset) => invoke('collations', { workspaceId, charset }),
  schemaDdl: (alter, name, charset, collation) => invoke('schema_ddl', { alter, name, charset, collation }),
  alterScript: (workspaceId, kind, schema, name) => invoke('alter_script', { workspaceId, kind, schema, name }),
  createTemplate: (kind, schema) => invoke('create_template', { kind, schema }),
  scriptOpen: () => invoke('script_open'),
  scriptSave: (path, suggestedName, text) => invoke('script_save', { path, suggestedName, text }),
  historyList: (search, limit) => invoke('history_list', { search, limit }),
  historyClear: () => invoke('history_clear'),
  selftestConfig: () => invoke('selftest_config'),
  selftestReport: (report) => invoke('selftest_report', { report }),
};

/** True inside the desktop app, false in a plain browser. */
export const inDesktopApp = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const backend: Backend = inDesktopApp ? tauriBackend : mockBackend;
