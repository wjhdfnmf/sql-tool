// Shapes shared with the Rust command layer. Field names match what the
// Rust side serialises (camelCase).

export type TlsSetting = 'disabled' | 'unverified' | 'verify-ca' | 'verify-identity';

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
  cleartextAuth: boolean;
  cleartextRiskAccepted: boolean;
  savePassword: boolean;
  color: string | null;
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
    cleartextAuth: false,
    cleartextRiskAccepted: false,
    savePassword: true,
    color: null,
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
  name: string;
  color: string | null;
  host: string;
  user: string;
  info: ServerInfo;
  security: 'verified' | 'unverified' | 'none';
  database: string | null;
}

export type ColumnKind = 'number' | 'text' | 'date' | 'json' | 'binary';

export interface ColumnInfo {
  name: string;
  kind: ColumnKind;
}

export type RunMode = { kind: 'all' } | { kind: 'at'; offset: number };

export type RunMsg =
  | { type: 'started'; index: number; total: number; sql: string }
  | { type: 'resultSet'; index: number; resultId: number; columns: ColumnInfo[] }
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
