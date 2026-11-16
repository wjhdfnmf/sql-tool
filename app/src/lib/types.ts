// Shapes shared with the Rust command layer. Field names match what the
// Rust side serialises (camelCase).

export type TlsSetting = 'disabled' | 'unverified' | 'verify-ca' | 'verify-identity' | 'pinned';

export interface Profile {
  /** Empty until the profile has been saved. */
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  database: string | null;
  tls: TlsSetting;
  caFile: string | null;
  /** SHA-256 of the certificate trusted in pinned mode, as hex. */
  pinnedSha256: string | null;
  cleartextAuth: boolean;
  cleartextRiskAccepted: boolean;
  savePassword: boolean;
  color: string | null;
  /** Refuse anything that is not a read. */
  readOnly: boolean;
  /** Ask before structure changes and before UPDATE or DELETE without WHERE. */
  production: boolean;
  /** Whether the keychain holds a password. Set by the backend. */
  hasPassword?: boolean;
}

export function newProfile(): Profile {
  return {
    id: '',
    name: '',
    host: '',
    port: 3306,
    user: '',
    database: null,
    tls: 'verify-identity',
    caFile: null,
    pinnedSha256: null,
    cleartextAuth: false,
    cleartextRiskAccepted: false,
    savePassword: true,
    color: null,
    readOnly: false,
    production: false,
  };
}

export interface ServerVersion {
  flavor: 'mysql' | 'mariadb' | 'percona';
  major: number;
  minor: number;
  patch: number;
  enterprise: boolean;
  raw: string;
}

export interface ServerInfo {
  version: ServerVersion;
  versionComment: string;
  connectionId: number;
  currentUser: string;
  tlsVersion: string | null;
  tlsCipher: string | null;
}

export interface WorkspaceInfo {
  id: number;
  /** The saved connection this came from; null for the demo server. */
  profileId: string | null;
  name: string;
  color: string | null;
  host: string;
  user: string;
  info: ServerInfo;
  security: 'verified' | 'pinned' | 'unverified' | 'none';
  database: string | null;
  readOnly: boolean;
  production: boolean;
}

export type ColumnKind = 'number' | 'text' | 'date' | 'json' | 'binary';

export interface ColumnInfo {
  name: string;
  kind: ColumnKind;
  /** The table column behind this result column; null for computed columns. */
  source: { schema: string; table: string; column: string } | null;
  primaryKey: boolean;
}

/** A server's certificate, shown before the user decides to trust it. */
export interface CertificateInfo {
  sha256: string;
  subject: string;
  issuer: string;
  notAfter: string;
  selfSigned: boolean;
}

/** `ab:cd:...` in groups, easier to compare by eye than 64 run-together digits. */
export function formatFingerprint(hex: string): string {
  return (hex.toUpperCase().match(/.{1,2}/g) ?? []).join(':');
}

/** Something to confirm before running on a production connection. */
export interface Concern {
  index: number;
  reason: string;
  sql: string;
}

export type ExportFormat = 'csv' | 'tsv' | 'json' | 'sql';

export interface Exported {
  path: string;
  rows: number;
  /** The result stopped at the row limit, so the file is not everything the query returns. */
  truncated: boolean;
  /** A filter was on, so only matching rows were written. */
  filtered: boolean;
}

/** A rectangle of cells in the rows a grid shows. Both ends are included. */
export interface CellRange {
  rowStart: number;
  rowEnd: number;
  colStart: number;
  colEnd: number;
}

/** `list` is quoted values separated by commas, for an IN (...) clause. */
export type CopyFormat = 'tsv' | 'csv' | 'sql' | 'list';

/** Totals for a selection. The number fields are null when it holds no numbers. */
export interface RangeStats {
  rows: number;
  cells: number;
  numbers: number;
  sum: number | null;
  min: number | null;
  max: number | null;
  average: number | null;
}

/** Where edits made in a result grid go. */
export interface EditTarget {
  schema: string;
  table: string;
  keyColumns: number[];
  /** Per result column: the table column it edits, or null if not editable. */
  columns: (string | null)[];
}

export interface CellChange {
  column: number;
  /** null sets the cell to NULL. */
  value: string | null;
}

export type RowEdit =
  | { type: 'update'; row: number; changes: CellChange[] }
  | { type: 'delete'; row: number }
  | { type: 'insert'; values: CellChange[] };

export interface ColumnSpec {
  name: string;
  dataType: string;
  nullable: boolean;
  default: string | null;
  onUpdate: string | null;
  autoIncrement: boolean;
  comment: string;
  collation: string | null;
  originalName: string | null;
  locked: boolean;
}

export interface IndexSpec {
  name: string;
  columns: string[];
  unique: boolean;
  originalName: string | null;
  locked: boolean;
}

export interface ForeignKeySpec {
  /** May be empty for a new key; the server then names it. */
  name: string;
  columns: string[];
  /** Empty means the table's own schema. */
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  /** RESTRICT, CASCADE, SET NULL, NO ACTION or SET DEFAULT; empty for the server's default. */
  onUpdate: string;
  onDelete: string;
  originalName: string | null;
}

export interface TableSpec {
  schema: string;
  name: string;
  columns: ColumnSpec[];
  primaryKey: string[];
  indexes: IndexSpec[];
  foreignKeys: ForeignKeySpec[];
  engine: string | null;
  collation: string | null;
  comment: string;
}

export interface ForeignKey {
  name: string;
  columns: string[];
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate: string;
  onDelete: string;
}

export interface TableKeys {
  primaryKey: string[];
  indexes: IndexSpec[];
  foreignKeys: ForeignKey[];
}

export type ObjectAction = 'drop' | 'dropSchema' | 'truncate' | 'rename';

export type RunMode = { kind: 'all' } | { kind: 'at'; offset: number } | { kind: 'explainAt'; offset: number };

export type RunMsg =
  | { type: 'started'; index: number; total: number; sql: string }
  | { type: 'resultSet'; index: number; resultId: number; columns: ColumnInfo[]; sql: string }
  | { type: 'progress'; resultId: number; rows: number }
  | { type: 'resultDone'; index: number; resultId: number; rows: number; truncated: boolean; elapsedMs: number }
  | {
      type: 'done';
      index: number;
      affectedRows: number;
      lastInsertId: number | null;
      info: string;
      warnings: number;
      elapsedMs: number;
    }
  | { type: 'failed'; index: number; message: string; elapsedMs: number }
  | { type: 'database'; name: string | null }
  | { type: 'reconnected' }
  | { type: 'finished'; ran: number; total: number; failed: boolean; elapsedMs: number };

export interface TableEntry {
  name: string;
  isView: boolean;
}

export interface SchemaObjects {
  tables: TableEntry[];
  routines: { name: string; kind: string }[];
  triggers: { name: string; table: string }[];
  events: string[];
}

export interface ColumnEntry {
  name: string;
  columnType: string;
  nullable: boolean;
  key: string;
  default: string | null;
  extra: string;
}

export type ObjectKind = 'table' | 'view' | 'procedure' | 'function' | 'trigger' | 'event';

export interface TableDetails {
  isView: boolean;
  engine: string | null;
  /** The storage engine's estimate, not an exact count. */
  rowsEstimate: number | null;
  collation: string | null;
  comment: string;
  created: string | null;
  dataBytes: number | null;
  indexBytes: number | null;
}

export interface SchemaDefaults {
  charset: string;
  collation: string;
}

export interface Charset {
  name: string;
  defaultCollation: string;
}

export interface ScriptFile {
  path: string;
  name: string;
  text: string;
}

/** Something picked in the schema tree. `name` is null for a schema itself. */
export interface TreeTarget {
  kind: 'schema' | ObjectKind;
  schema: string;
  name: string | null;
}

export interface HistoryEntry {
  atMs: number;
  profileName: string;
  sql: string;
  durationMs: number;
  ok: boolean;
  summary: string;
}

export interface CellValue {
  /** null for SQL NULL. */
  text: string | null;
  bytes: number;
  truncated: boolean;
}

/** What the Settings screen changes. Stored by the backend, which corrects values outside their range. */
export interface Settings {
  /** system, light, dark, midnight, sepia or contrast. */
  theme: string;
  /** Size of everything in the window, in percent. */
  zoom: number;
  editorTabSize: number;
  editorWordWrap: boolean;
  uppercaseKeywords: boolean;
  rememberTabs: boolean;
  /** Rows kept per result for a new connection; null means no limit. */
  defaultRowLimit: number | null;
  /** 0 means a query is never stopped for running long. */
  queryTimeoutSecs: number;
  connectTimeoutSecs: number;
  /** 0 means idle connections are not pinged. */
  keepAliveSecs: number;
  keepHistory: boolean;
  resultMemoryMb: number;
}

export interface AppInfo {
  version: string;
  devtools: boolean;
}

export interface SelfTestConfig {
  profile: Profile;
  password: string;
  schema: string;
  table: string;
  bigQuery: string;
}

/** What a failed backend call rejects with. */
export interface UiError {
  kind: string;
  message: string;
}

export function asUiError(e: unknown): UiError {
  if (e && typeof e === 'object' && 'message' in e) {
    const o = e as { kind?: unknown; message: unknown };
    return { kind: typeof o.kind === 'string' ? o.kind : 'other', message: String(o.message) };
  }
  return { kind: 'other', message: String(e) };
}

/** Adds a next step to connection errors the user can fix in the connection settings. */
export function explainConnectError(error: UiError): string {
  switch (error.kind) {
    case 'tls':
      return `${error.message}. The server's certificate could not be verified. Servers with a default or self-signed certificate can be reached by pinning their certificate, with "Encrypted, not verified" (what most MySQL tools use), or with their CA file.`;
    case 'certificate-changed':
      return `${error.message}.`;
    case 'cleartext-not-enabled':
      return `${error.message}. Turn on "Allow cleartext authentication" for this connection (LDAP and PAM accounts need it).`;
    case 'no-tls':
      return `${error.message}. Choose "Not encrypted" only if that is acceptable on this network.`;
    default:
      return error.message;
  }
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

export function serverLabel(v: ServerVersion): string {
  const name =
    v.flavor === 'mariadb' ? 'MariaDB' : v.flavor === 'percona' ? 'Percona Server' : v.enterprise ? 'MySQL Enterprise' : 'MySQL';
  return `${name} ${v.major}.${v.minor}.${v.patch}`;
}

export function quoteIdentifier(name: string): string {
  return '`' + name.replaceAll('`', '``') + '`';
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(2)} s`;
  const minutes = Math.floor(ms / 60_000);
  return `${minutes} min ${Math.round((ms % 60_000) / 1000)} s`;
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-US');
}
