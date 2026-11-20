// The single doorway from the UI to the Rust side. Components never call
// Tauri directly, which is what lets the UI also run in a plain browser
// against the mock backend during development.

import { Channel, invoke } from '@tauri-apps/api/core';
import type { ChartData, ChartRequest, ChartRole, ColumnSummary } from './chart';
import { mockBackend } from './mock';
import type {
  AppInfo,
  CertificateInfo,
  CellRange,
  CellValue,
  Charset,
  ColumnEntry,
  Concern,
  CopyFormat,
  EditTarget,
  Exported,
  ExportFormat,
  RangeStats,
  HistoryEntry,
  ObjectAction,
  ObjectKind,
  RowEdit,
  TableKeys,
  TableSpec,
  Profile,
  RunMode,
  RunMsg,
  SchemaDefaults,
  SchemaObjects,
  ScriptFile,
  SelfTestConfig,
  ServerInfo,
  Settings,
  SlicePlan,
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
  /** Fetches the certificate a server presents. Sends no credentials. */
  certificateProbe(host: string, port: number): Promise<CertificateInfo>;
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
  /** How many slices a range makes, and the first one's statement. Rejects with the reason if the query cannot be sliced. */
  slicePlan(sql: string, start: string, end: string, stepSecs: number): Promise<SlicePlan>;
  /**
   * Runs a query once for each slice of a time range and puts the rows end to end as one result. The query marks its
   * range with {start} and {end}. Progress arrives through `onEvent`, as for `run`, with `slices` messages as well.
   */
  runSliced(
    workspaceId: number,
    tabId: string,
    sql: string,
    start: string,
    end: string,
    stepSecs: number,
    maxRows: number | null,
    onEvent: (message: RunMsg) => void,
  ): Promise<void>;
  /** Makes a schema the default for every tab of the connection. Rejects if it does not exist. */
  setDefaultSchema(workspaceId: number, schema: string): Promise<void>;
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
  scriptOpen(workspaceId: number): Promise<ScriptFile | null>;
  /** Where Open and Save As start for this connection: its script folder, if it has one that exists. */
  scriptFolderOf(workspaceId: number): Promise<string | null>;
  /** Shows the native folder dialog, for a connection's script folder. Null if the user cancels. */
  folderPick(start: string | null): Promise<string | null>;
  /** Saves to `path`, or asks where if `path` is null. Null if the user cancels. */
  scriptSave(workspaceId: number, path: string | null, suggestedName: string, text: string): Promise<ScriptFile | null>;
  /** What a production connection should confirm before running this. */
  reviewScript(script: string, mode: RunMode): Promise<Concern[]>;
  /** Sorts and filters what the grid shows; resolves to the visible row count. */
  resultView(resultId: number, filter: string | null, sortColumn: number | null, descending: boolean): Promise<number>;
  /** Asks where to save and writes the visible rows. Null if cancelled. */
  exportResult(resultId: number, format: ExportFormat, table: string): Promise<Exported | null>;
  /**
   * Asks where to save, then runs the statement again and writes every row it returns, however many,
   * regardless of the row limit. Runs in a connection of its own under `tabId`; `cancel` with that id stops it.
   * `onProgress` gets the number of rows written so far. Null if the user cancels the file dialog.
   */
  exportQuery(
    workspaceId: number,
    tabId: string,
    sql: string,
    format: ExportFormat,
    table: string,
    onProgress: (rows: number) => void,
  ): Promise<Exported | null>;
  /** The cells of a selection as text for the clipboard, built from the stored rows. */
  copyRange(resultId: number, range: CellRange, format: CopyFormat, headers: boolean, table: string): Promise<string>;
  /** Count, sum, average, smallest and largest of a selection. */
  rangeStats(resultId: number, range: CellRange): Promise<RangeStats>;
  /**
   * What a chart of a finished result shows, worked out by the backend from every row of it.
   * Rejects with kind `chart` and the reason if the columns chosen cannot make that chart.
   */
  chartData(resultId: number, request: ChartRequest): Promise<ChartData>;
  /** What one column holds when read in `role`: its range, or its most frequent values. */
  chartColumn(resultId: number, column: number, role: ChartRole): Promise<ColumnSummary>;
  /** Lets go of what the backend kept for a result's chart. */
  chartClose(resultId: number): Promise<void>;
  /** Asks where to save, then writes a PNG picture of a chart. Resolves to the path, or null if cancelled. */
  chartSaveImage(png: Uint8Array): Promise<string | null>;
  /** Rejects with kind `not-editable` and the reason if the result cannot be edited. */
  editTarget(workspaceId: number, resultId: number): Promise<EditTarget>;
  previewEdits(resultId: number, edits: RowEdit[]): Promise<string[]>;
  /** Applies the edits in one transaction; resolves to the number of statements run. */
  applyEdits(workspaceId: number, tabId: string, resultId: number, edits: RowEdit[]): Promise<number>;
  tableKeys(workspaceId: number, schema: string, table: string): Promise<TableKeys | null>;
  tableSpec(workspaceId: number, schema: string, table: string): Promise<TableSpec | null>;
  /** CREATE TABLE when `original` is null, else the ALTER TABLE between the two. */
  tableDdl(original: TableSpec | null, updated: TableSpec): Promise<string>;
  objectDdl(action: ObjectAction, kind: ObjectKind, schema: string, name: string, newName: string | null): Promise<string>;
  /** [table, column] pairs of a schema, for completion. */
  completionColumns(workspaceId: number, schema: string): Promise<[string, string][]>;
  historyList(search: string | null, limit: number): Promise<HistoryEntry[]>;
  historyClear(): Promise<void>;
  settingsGet(): Promise<Settings>;
  /** Saves the settings; resolves to them as stored, with out-of-range values corrected. */
  settingsSet(settings: Settings): Promise<Settings>;
  selftestConfig(): Promise<SelfTestConfig | null>;
  selftestReport(report: string): Promise<void>;
}

const tauriBackend: Backend = {
  appInfo: () => invoke('app_info'),
  profilesList: () => invoke('profiles_list'),
  profileSave: (profile, password) => invoke('profile_save', { profile, password }),
  profileDelete: (id) => invoke('profile_delete', { id }),
  connectionTest: (profile, password) => invoke('connection_test', { profile, password }),
  certificateProbe: (host, port) => invoke('certificate_probe', { host, port }),
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
  slicePlan: (sql, start, end, stepSecs) => invoke('slice_plan', { sql, start, end, stepSecs }),
  runSliced: (workspaceId, tabId, sql, start, end, stepSecs, maxRows, onEvent) => {
    const channel = new Channel<RunMsg>();
    channel.onmessage = onEvent;
    return invoke('run_sliced', { workspaceId, tabId, sql, start, end, stepSecs, maxRows, onEvent: channel });
  },
  setDefaultSchema: (workspaceId, schema) => invoke('set_default_schema', { workspaceId, schema }),
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
  scriptOpen: (workspaceId) => invoke('script_open', { workspaceId }),
  folderPick: (start) => invoke('folder_pick', { start }),
  scriptFolderOf: (workspaceId) => invoke('script_folder_of', { workspaceId }),
  scriptSave: (workspaceId, path, suggestedName, text) => invoke('script_save', { workspaceId, path, suggestedName, text }),
  reviewScript: (script, mode) => invoke('review_script', { script, mode }),
  resultView: (resultId, filter, sortColumn, descending) => invoke('result_view', { resultId, filter, sortColumn, descending }),
  exportResult: (resultId, format, table) => invoke('export_result', { resultId, format, table }),
  exportQuery: (workspaceId, tabId, sql, format, table, onProgress) => {
    const channel = new Channel<number>();
    channel.onmessage = onProgress;
    return invoke('export_query', { workspaceId, tabId, sql, format, table, onProgress: channel });
  },
  copyRange: (resultId, range, format, headers, table) => invoke('copy_range', { resultId, range, format, headers, table }),
  rangeStats: (resultId, range) => invoke('range_stats', { resultId, range }),
  chartData: (resultId, request) => invoke('chart_data', { resultId, request }),
  chartColumn: (resultId, column, role) => invoke('chart_column', { resultId, column, role }),
  chartClose: (resultId) => invoke('chart_close', { resultId }),
  // The picture goes as the raw body of the call, not as JSON.
  chartSaveImage: (png) => invoke('chart_save_image', png),
  editTarget: (workspaceId, resultId) => invoke('edit_target', { workspaceId, resultId }),
  previewEdits: (resultId, edits) => invoke('preview_edits', { resultId, edits }),
  applyEdits: (workspaceId, tabId, resultId, edits) => invoke('apply_edits', { workspaceId, tabId, resultId, edits }),
  tableKeys: (workspaceId, schema, table) => invoke('table_keys', { workspaceId, schema, table }),
  tableSpec: (workspaceId, schema, table) => invoke('table_spec', { workspaceId, schema, table }),
  tableDdl: (original, updated) => invoke('table_ddl', { original, updated }),
  objectDdl: (action, kind, schema, name, newName) => invoke('object_ddl', { action, kind, schema, name, newName }),
  completionColumns: (workspaceId, schema) => invoke('completion_columns', { workspaceId, schema }),
  historyList: (search, limit) => invoke('history_list', { search, limit }),
  historyClear: () => invoke('history_clear'),
  settingsGet: () => invoke('settings_get'),
  settingsSet: (settings) => invoke('settings_set', { settings }),
  selftestConfig: () => invoke('selftest_config'),
  selftestReport: (report) => invoke('selftest_report', { report }),
};

/** True inside the desktop app, false in a plain browser. */
export const inDesktopApp = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const backend: Backend = inDesktopApp ? tauriBackend : mockBackend;
