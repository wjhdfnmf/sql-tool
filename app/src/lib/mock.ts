// A stand-in backend for running the UI in a plain browser (`npm run dev`).
// It fakes a server with a few schemas and synthetic results, in the same
// wire format the real backend uses, so every screen can be exercised
// without the desktop shell or a database.

import type { Backend } from './backend';
import { mockChartClose, mockChartColumn, mockChartData } from './mockChart';
import type { ColumnInfo, HistoryEntry, Profile, RunMsg, ServerInfo, Settings, UiError, WorkspaceInfo } from './types';

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const fail = (kind: string, message: string): UiError => ({ kind, message });

let profiles: Profile[] = [
  {
    id: 'p1',
    name: 'Reporting (LDAP)',
    host: 'reports.db.example.test',
    port: 3306,
    user: 'ann',
    database: 'sales',
    tls: 'unverified',
    caFile: null,
    pinnedSha256: null,
    cleartextAuth: true,
    cleartextRiskAccepted: true,
    savePassword: true,
    color: null,
    readOnly: false,
    production: false,
    scriptFolder: null,
    hasPassword: true,
  },
  {
    id: 'p2',
    name: 'Orders production',
    host: 'orders-prod.db.example.test',
    port: 3306,
    user: 'ann',
    database: null,
    tls: 'verify-identity',
    caFile: null,
    pinnedSha256: null,
    cleartextAuth: true,
    cleartextRiskAccepted: false,
    savePassword: false,
    color: 'red',
    readOnly: false,
    production: true,
    scriptFolder: null,
    hasPassword: false,
  },
  {
    id: 'p3',
    name: 'Local MariaDB',
    host: '127.0.0.1',
    port: 3307,
    user: 'root',
    database: null,
    tls: 'disabled',
    caFile: null,
    pinnedSha256: null,
    cleartextAuth: false,
    cleartextRiskAccepted: false,
    savePassword: true,
    color: 'green',
    readOnly: false,
    production: false,
    scriptFolder: null,
    hasPassword: true,
  },
];

const history: HistoryEntry[] = [
  { atMs: Date.now() - 60_000, profileName: 'Reporting (LDAP)', sql: 'SELECT * FROM sales.orders WHERE status = \'open\'', durationMs: 42, ok: true, summary: '1,204 rows' },
  { atMs: Date.now() - 3_600_000, profileName: 'Reporting (LDAP)', sql: 'UPDATE sales.customers SET tier = \'gold\' WHERE id = 7', durationMs: 8, ok: true, summary: '1 rows affected' },
  { atMs: Date.now() - 86_400_000, profileName: 'Local MariaDB', sql: 'SELEC 1', durationMs: 1, ok: false, summary: 'server error 1064 (42000): You have an error in your SQL syntax' },
];

const serverInfo = (profile: Profile | null): ServerInfo => ({
  version: { flavor: 'mysql', major: 8, minor: 4, patch: 3, enterprise: true, raw: '8.4.3-commercial' },
  versionComment: 'MySQL Enterprise Server - Commercial',
  connectionId: 4211,
  currentUser: `${profile?.user ?? 'demo'}@%`,
  tlsVersion: profile?.tls === 'disabled' ? null : 'TLSv1.3',
  tlsCipher: profile?.tls === 'disabled' ? null : 'TLS_AES_256_GCM_SHA384',
});

const schemas: Record<string, Record<string, [string, string, boolean, string][]>> = {
  sales: {
    customers: [
      ['id', 'int unsigned', false, 'PRI'],
      ['name', 'varchar(120)', false, ''],
      ['email', 'varchar(200)', true, 'UNI'],
      ['tier', "enum('basic','gold')", false, ''],
      ['created_at', 'timestamp', false, ''],
    ],
    orders: [
      ['id', 'bigint unsigned', false, 'PRI'],
      ['customer_id', 'int unsigned', false, 'MUL'],
      ['status', 'varchar(20)', false, 'MUL'],
      ['total', 'decimal(12,2)', false, ''],
      ['placed_at', 'datetime', false, ''],
      ['notes', 'text', true, ''],
    ],
    order_lines: [
      ['order_id', 'bigint unsigned', false, 'PRI'],
      ['line_no', 'smallint', false, 'PRI'],
      ['sku', 'varchar(40)', false, ''],
      ['quantity', 'int', false, ''],
    ],
  },
  inventory: {
    products: [
      ['sku', 'varchar(40)', false, 'PRI'],
      ['title', 'varchar(200)', false, ''],
      ['stock', 'int', false, ''],
      ['attributes', 'json', true, ''],
    ],
  },
  information_schema: {},
  mysql: {},
};

const mockDefaults: Settings = {
  theme: 'system', zoom: 100, editorTabSize: 2, editorWordWrap: false, uppercaseKeywords: true, rememberTabs: true,
  defaultRowLimit: 10_000, queryTimeoutSecs: 0, connectTimeoutSecs: 15, keepAliveSecs: 0, keepHistory: true, resultMemoryMb: 512,
};

let nextId = 100;
const results = new Map<number, { columns: ColumnInfo[]; rows: number }>();
const cancelled = new Set<string>();

const demoColumns: ColumnInfo[] = [
  { name: 'id', kind: 'number', source: { schema: 'sales', table: 'orders', column: 'id' }, primaryKey: true },
  { name: 'customer', kind: 'text', source: { schema: 'sales', table: 'orders', column: 'customer' }, primaryKey: false },
  { name: 'status', kind: 'text', source: { schema: 'sales', table: 'orders', column: 'status' }, primaryKey: false },
  { name: 'total', kind: 'number', source: { schema: 'sales', table: 'orders', column: 'total' }, primaryKey: false },
  { name: 'placed_at', kind: 'date', source: { schema: 'sales', table: 'orders', column: 'placed_at' }, primaryKey: false },
  { name: 'notes', kind: 'text', source: { schema: 'sales', table: 'orders', column: 'notes' }, primaryKey: false },
  { name: 'attributes', kind: 'json', source: { schema: 'sales', table: 'orders', column: 'attributes' }, primaryKey: false },
  { name: 'receipt', kind: 'binary', source: { schema: 'sales', table: 'orders', column: 'receipt' }, primaryKey: false },
];

function cellText(row: number, col: number): string | null {
  switch (col) {
    case 0: return String(row + 1);
    case 1: return `Customer ${(row * 7919) % 9973}`;
    case 2: return ['open', 'paid', 'shipped', 'cancelled'][row % 4];
    case 3: return `${(row * 37) % 100000}.${String(row % 100).padStart(2, '0')}`;
    case 4: return `2026-${String((row % 12) + 1).padStart(2, '0')}-${String((row % 28) + 1).padStart(2, '0')} ${String(row % 24).padStart(2, '0')}:${String(row % 60).padStart(2, '0')}:00`;
    case 5: return row % 10 === 0 ? null : row % 7 === 0 ? `A much longer note for order ${row + 1}, written to check that long text is cut off cleanly inside the cell and shown in full in the value viewer.` : `note ${row + 1}`;
    case 6: return row % 3 === 0 ? null : `{"gift": ${row % 2 === 0}, "tags": ["a", "b"], "row": ${row}}`;
    default: return 'PDF-1.7 binary receipt data '.repeat(4);
  }
}

function encodeRows(columns: number, offset: number, count: number, total: number): Uint8Array {
  const encoder = new TextEncoder();
  const last = Math.min(total, offset + count);
  const chunks: number[] = [];
  const n = Math.max(0, last - offset);
  chunks.push(n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff, (n >> 24) & 0xff);
  for (let r = offset; r < last; r++) {
    for (let c = 0; c < columns; c++) {
      const text = cellText(r, c % demoColumns.length);
      if (text === null) {
        chunks.push(0xfb);
        continue;
      }
      const bytes = encoder.encode(text);
      if (bytes.length < 251) chunks.push(bytes.length);
      else chunks.push(0xfc, bytes.length & 0xff, bytes.length >> 8);
      for (const b of bytes) chunks.push(b);
    }
  }
  return Uint8Array.from(chunks);
}

function splitNaive(script: string): string[] {
  return script.split(';').map((s) => s.trim()).filter(Boolean);
}

export const mockBackend: Backend = {
  appInfo: async () => ({ version: '0.0.1-mock', devtools: true }),
  profilesList: async () => structuredClone(profiles),
  profileSave: async (profile, password) => {
    const saved = { ...profile, id: profile.id || `p${nextId++}` };
    saved.hasPassword = saved.savePassword && (password !== null || !!profiles.find((p) => p.id === saved.id)?.hasPassword);
    profiles = profiles.some((p) => p.id === saved.id) ? profiles.map((p) => (p.id === saved.id ? saved : p)) : [...profiles, saved];
    return structuredClone(saved);
  },
  profileDelete: async (id) => {
    profiles = profiles.filter((p) => p.id !== id);
  },
  connectionTest: async (profile) => {
    await wait(500);
    if (profile.host.includes('bad')) throw fail('network', 'network error: No such host is known.');
    if (profile.host.includes('selfsigned') && profile.tls.startsWith('verify')) {
      throw fail('tls', 'TLS failed: invalid peer certificate: UnknownIssuer');
    }
    return serverInfo(profile);
  },
  certificateProbe: async (host) => {
    await wait(400);
    if (host.includes('bad')) throw fail('network', 'network error: No such host is known.');
    return {
      sha256: '3f9a1c0e5b7d42a8916ce0d4f1b2a37c58e94d6072ab13cf8e5d09b6a4c7f210',
      subject: 'CN=MySQL_Server_8.4.3_Auto_Generated_Server_Certificate',
      issuer: 'CN=MySQL_Server_8.4.3_Auto_Generated_CA_Certificate',
      notAfter: 'Mar 14 09:12:44 2034 +00:00',
      selfSigned: false,
    };
  },
  connect: async (profileId, password) => {
    await wait(400);
    const profile = profiles.find((p) => p.id === profileId);
    if (!profile) throw fail('no-profile', 'This connection no longer exists.');
    if (password === null && !profile.hasPassword) throw fail('password-required', 'A password is needed for this connection.');
    if (password === 'wrong') throw fail('access-denied', "access denied: Access denied for user 'ann'@'10.0.0.5' (using password: YES) (error 1045)");
    return {
      id: nextId++,
      profileId: profile.id,
      name: profile.name,
      color: profile.color,
      host: `${profile.host}:${profile.port}`,
      user: profile.user,
      info: serverInfo(profile),
      security: profile.tls === 'disabled' ? 'none' : profile.tls === 'unverified' ? 'unverified' : profile.tls === 'pinned' ? 'pinned' : 'verified',
      database: profile.database,
      readOnly: profile.readOnly,
      production: profile.production,
    } satisfies WorkspaceInfo;
  },
  connectDemo: async () => ({
    id: nextId++,
    profileId: null,
    name: 'Demo server',
    color: null,
    host: '127.0.0.1:53211',
    user: 'demo',
    info: serverInfo(null),
    security: 'verified',
    database: null,
    readOnly: false,
    production: false,
  }),
  disconnect: async () => {},
  closeTab: async () => {},
  run: async (_workspaceId, tabId, script, mode, maxRows, onEvent) => {
    let statements = splitNaive(script);
    if (mode.kind !== 'all') {
      let position = 0;
      const index = script.split(';').findIndex((part) => (position += part.length + 1) > mode.offset);
      statements = statements.slice(Math.max(0, Math.min(index, statements.length - 1))).slice(0, 1);
    }
    const started = performance.now();
    cancelled.delete(tabId);
    let failed = false;
    let ran = 0;
    for (const [index, sql] of statements.entries()) {
      const t0 = performance.now();
      const elapsed = () => Math.round(performance.now() - t0);
      onEvent({ type: 'started', index, total: statements.length, sql });
      ran++;
      const lower = mode.kind === 'explainAt' ? `explain ${sql.toLowerCase()} limit 3` : sql.toLowerCase();
      if (lower.includes('sleep')) {
        for (let i = 0; i < 100 && !cancelled.has(tabId); i++) await wait(100);
        if (cancelled.has(tabId)) {
          onEvent({ type: 'failed', index, message: 'server error 1317 (70100): Query execution was interrupted', elapsedMs: elapsed() });
          failed = true;
          break;
        }
      }
      if (/^selec\b|\bnosuch\b/.test(lower)) {
        await wait(60);
        onEvent({ type: 'failed', index, message: "server error 1064 (42000): You have an error in your SQL syntax; check the manual that corresponds to your MySQL server version for the right syntax to use near '" + sql.slice(0, 20) + "' at line 1", elapsedMs: elapsed() });
        failed = true;
        break;
      }
      if (/^(select|show|with|explain|describe|call)\b/.test(lower)) {
        const wanted = lower.includes('big') ? 1_000_000 : Number(/limit\s+(\d+)/.exec(lower)?.[1] ?? 2500);
        const total = maxRows === null ? wanted : Math.min(wanted, maxRows);
        const columns = lower.includes('wide') ? Array.from({ length: 40 }, (_, i) => ({ ...demoColumns[i % 8], name: `${demoColumns[i % 8].name}_${i}` })) : demoColumns;
        const resultId = nextId++;
        results.set(resultId, { columns, rows: 0 });
        onEvent({ type: 'resultSet', index, resultId, columns, sql });
        for (let rows = Math.min(256, total); ; rows = Math.min(total, rows + 40_000)) {
          await wait(30);
          results.get(resultId)!.rows = rows;
          onEvent({ type: 'progress', resultId, rows });
          if (rows >= total) break;
        }
        onEvent({ type: 'resultDone', index, resultId, rows: total, truncated: total < wanted, elapsedMs: elapsed() });
      } else {
        await wait(40);
        if (lower.startsWith('use ')) onEvent({ type: 'database', name: sql.slice(4).replaceAll('`', '').trim() });
        onEvent({ type: 'done', index, affectedRows: lower.startsWith('use') || lower.startsWith('set') ? 0 : 3, lastInsertId: null, info: '', warnings: 0, elapsedMs: elapsed() });
      }
    }
    onEvent({ type: 'finished', ran, total: statements.length, failed, elapsedMs: Math.round(performance.now() - started) });
  },
  cancel: async (_workspaceId, tabId) => {
    cancelled.add(tabId);
  },
  setDefaultSchema: async (_workspaceId, schema) => {
    await wait(80);
    if (!(schema in schemas)) throw fail('server', `server error 1049 (42000): Unknown database '${schema}'`);
  },
  getRows: async (resultId, offset, count) => {
    await wait(4);
    const result = results.get(resultId);
    if (!result) throw fail('no-result', 'This result is no longer available.');
    return encodeRows(result.columns.length, offset, count, result.rows);
  },
  getCell: async (_resultId, row, column) => {
    const text = cellText(row, column % demoColumns.length);
    return { text, bytes: text === null ? 0 : new TextEncoder().encode(text).length, truncated: false };
  },
  closeResults: async (ids) => {
    for (const id of ids) {
      results.delete(id);
      mockChartClose(id);
    }
  },
  schemaList: async () => {
    await wait(150);
    return Object.keys(schemas);
  },
  schemaObjects: async (_workspaceId, schema) => {
    await wait(200);
    const tables = Object.keys(schemas[schema] ?? {}).map((name) => ({ name, isView: false }));
    if (schema === 'sales') tables.push({ name: 'open_orders', isView: true });
    return {
      tables,
      routines: schema === 'sales' ? [{ name: 'close_month', kind: 'PROCEDURE' }, { name: 'order_total', kind: 'FUNCTION' }] : [],
      triggers: schema === 'sales' ? [{ name: 'orders_audit', table: 'orders' }] : [],
      events: [],
    };
  },
  tableColumns: async (_workspaceId, schema, table) => {
    await wait(120);
    return (schemas[schema]?.[table] ?? []).map(([name, columnType, nullable, key]) => ({
      name,
      columnType,
      nullable,
      key,
      default: null,
      extra: name === 'id' ? 'auto_increment' : '',
    }));
  },
  tableDetails: async (_workspaceId, schema, table) => {
    await wait(80);
    if (table === 'open_orders') {
      return { isView: true, engine: null, rowsEstimate: null, collation: null, comment: 'VIEW', created: null, dataBytes: null, indexBytes: null };
    }
    if (!schemas[schema]?.[table]) return null;
    return {
      isView: false,
      engine: 'InnoDB',
      rowsEstimate: table === 'orders' ? 1_204_331 : 8_412,
      collation: 'utf8mb4_0900_ai_ci',
      comment: table === 'orders' ? 'One row per customer order' : '',
      created: '2024-03-18 09:12:44',
      dataBytes: table === 'orders' ? 212_336_640 : 1_589_248,
      indexBytes: table === 'orders' ? 48_234_496 : 262_144,
    };
  },
  schemaDefaults: async () => ({ charset: 'utf8mb4', collation: 'utf8mb4_0900_ai_ci' }),
  charsets: async () => [
    { name: 'latin1', defaultCollation: 'latin1_swedish_ci' },
    { name: 'utf8mb3', defaultCollation: 'utf8mb3_general_ci' },
    { name: 'utf8mb4', defaultCollation: 'utf8mb4_0900_ai_ci' },
  ],
  collations: async (_workspaceId, charset) =>
    charset === 'utf8mb4'
      ? ['utf8mb4_0900_ai_ci', 'utf8mb4_bin', 'utf8mb4_general_ci', 'utf8mb4_unicode_ci']
      : [`${charset}_bin`, `${charset}_general_ci`],
  schemaDdl: async (alter, name, charset, collation) => {
    if (name.trim() === '') throw fail('other', 'Enter a name.');
    return (
      `${alter ? 'ALTER' : 'CREATE'} SCHEMA \`${name.trim()}\`` +
      (charset ? ` DEFAULT CHARACTER SET ${charset}` : '') +
      (collation ? ` DEFAULT COLLATE ${collation}` : '') +
      ';'
    );
  },
  alterScript: async (_workspaceId, kind, schema, name) => {
    await wait(120);
    if (kind === 'table') {
      return `-- Current definition of \`${schema}\`.\`${name}\`, for reference:\n--   CREATE TABLE \`${name}\` (\n--     \`id\` int NOT NULL AUTO_INCREMENT,\n--     PRIMARY KEY (\`id\`)\n--   )\n\nALTER TABLE \`${schema}\`.\`${name}\`\n  -- for example: ADD COLUMN \`new_column\` VARCHAR(100) NULL\n;\n`;
    }
    if (kind === 'view') return `USE \`${schema}\`;\n\nCREATE OR REPLACE VIEW \`${name}\` AS select 1;\n`;
    return `USE \`${schema}\`;\n\nDROP ${kind.toUpperCase()} IF EXISTS \`${name}\`;\n\nDELIMITER $$\nCREATE ${kind.toUpperCase()} \`${name}\`()\nBEGIN\n  SELECT 1;\nEND$$\nDELIMITER ;\n`;
  },
  createTemplate: async (kind, schema) => {
    const prefix = schema ? `\`${schema}\`.` : '';
    if (kind === 'table') return `CREATE TABLE ${prefix}\`new_table\` (\n  \`id\` INT NOT NULL AUTO_INCREMENT,\n  \`name\` VARCHAR(100) NOT NULL,\n  PRIMARY KEY (\`id\`)\n);\n`;
    if (kind === 'view') return `CREATE VIEW ${prefix}\`new_view\` AS\nSELECT *\nFROM ${prefix}\`table_name\`;\n`;
    return `DELIMITER $$\nCREATE ${kind.toUpperCase()} \`new_${kind}\`()\nBEGIN\n  SELECT 1;\nEND$$\nDELIMITER ;\n`;
  },
  scriptOpen: async () => ({
    path: 'C:\\scripts\\monthly-report.sql',
    name: 'monthly-report.sql',
    text: '-- Monthly report\nselect status, count(*) as orders, sum(total) as revenue from sales.orders where placed_at >= \'2026-10-01\' group by status order by revenue desc;\n',
  }),
  folderPick: async () => 'C:\\scripts',
  scriptFolderOf: async () => null,
  scriptSave: async (_workspaceId, path, suggestedName) => ({
    path: path ?? `C:\\scripts\\${suggestedName}`,
    name: path ? path.split('\\').pop()! : suggestedName,
    text: '',
  }),
  reviewScript: async (script) =>
    splitNaive(script)
      .map((sql, index) => {
        const lower = sql.toLowerCase();
        const first = lower.split(/\s+/)[0].toUpperCase();
        if (/^(drop|truncate|alter|create|rename|grant|revoke)\b/.test(lower)) {
          return { index, reason: `${first} changes structure, accounts or server settings`, sql };
        }
        if (/^(update|delete)\b/.test(lower) && !/\bwhere\b/.test(lower)) {
          return { index, reason: `${first} without a WHERE clause affects every row`, sql };
        }
        return null;
      })
      .filter((c) => c !== null),
  resultView: async (resultId, filter) => {
    const rows = results.get(resultId)?.rows ?? 0;
    return filter ? Math.ceil(rows / 3) : rows;
  },
  exportResult: async (resultId, format) => {
    await wait(300);
    return { path: `C:\\exports\\result.${format}`, rows: results.get(resultId)?.rows ?? 0, truncated: false, filtered: false };
  },
  exportQuery: async (_workspaceId, tabId, _sql, format, _table, onProgress) => {
    // Pretends to stream six million rows.
    cancelled.delete(tabId);
    for (let rows = 500_000; rows <= 6_000_000; rows += 500_000) {
      await wait(250);
      if (cancelled.has(tabId)) throw fail('stopped', 'Export stopped. The incomplete file was removed.');
      onProgress(rows);
    }
    return { path: `C:\\exports\\result.${format}`, rows: 6_000_000, truncated: false, filtered: false };
  },
  copyRange: async (resultId, range, format, headers, table) => {
    const result = results.get(resultId);
    if (!result) throw fail('no-result', 'This result is no longer available.');
    const lastRow = Math.min(range.rowEnd, result.rows - 1, range.rowStart + 4999);
    const lastCol = Math.min(range.colEnd, result.columns.length - 1);
    const cols: number[] = [];
    for (let c = range.colStart; c <= lastCol; c++) cols.push(c);
    const value = (row: number, c: number) => cellText(row, c % demoColumns.length);
    const literal = (row: number, c: number) => {
      const text = value(row, c);
      if (text === null) return 'NULL';
      return result.columns[c].kind === 'number' ? text : `'${text.replaceAll("'", "''")}'`;
    };
    const separator = format === 'csv' ? ',' : '\t';
    const lines: string[] = [];
    if (headers && (format === 'tsv' || format === 'csv')) lines.push(cols.map((c) => result.columns[c].name).join(separator));
    const list: string[] = [];
    for (let row = range.rowStart; row <= lastRow; row++) {
      if (format === 'sql') {
        const names = cols.map((c) => '`' + result.columns[c].name + '`').join(', ');
        lines.push(`INSERT INTO \`${table || 'table_name'}\` (${names}) VALUES (${cols.map((c) => literal(row, c)).join(', ')});`);
      } else if (format === 'list') {
        list.push(...cols.map((c) => literal(row, c)));
      } else {
        lines.push(cols.map((c) => value(row, c) ?? 'NULL').join(separator));
      }
    }
    return format === 'list' ? list.join(', ') : lines.join('\n');
  },
  rangeStats: async (resultId, range) => {
    const result = results.get(resultId);
    const stats = { rows: 0, cells: 0, numbers: 0, sum: null as number | null, min: null as number | null, max: null as number | null, average: null as number | null };
    if (!result) return stats;
    const lastRow = Math.min(range.rowEnd, result.rows - 1);
    const lastCol = Math.min(range.colEnd, result.columns.length - 1);
    stats.rows = Math.max(0, lastRow - range.rowStart + 1);
    stats.cells = stats.rows * Math.max(0, lastCol - range.colStart + 1);
    let sum = 0;
    // The mock adds up only the first rows; the real backend reads them all.
    for (let row = range.rowStart; row <= Math.min(lastRow, range.rowStart + 4999); row++) {
      for (let c = range.colStart; c <= lastCol; c++) {
        const text = cellText(row, c % demoColumns.length);
        const n = text === null || text.trim() === '' ? NaN : Number(text);
        if (!Number.isFinite(n)) continue;
        stats.numbers++;
        sum += n;
        stats.min = stats.min === null ? n : Math.min(stats.min, n);
        stats.max = stats.max === null ? n : Math.max(stats.max, n);
      }
    }
    if (stats.numbers > 0) {
      stats.sum = sum;
      stats.average = sum / stats.numbers;
    }
    return stats;
  },
  chartData: async (resultId, request) => {
    await wait(30);
    const result = results.get(resultId);
    if (!result) throw fail('no-result', 'This result is no longer available.');
    return mockChartData(resultId, (row, column) => cellText(row, column % demoColumns.length), result.rows, request);
  },
  chartColumn: async (resultId, column, role) => {
    await wait(30);
    const result = results.get(resultId);
    if (!result) throw fail('no-result', 'This result is no longer available.');
    return mockChartColumn(resultId, (row, c) => cellText(row, c % demoColumns.length), result.rows, column, role);
  },
  chartClose: async (resultId) => mockChartClose(resultId),
  chartSaveImage: async () => 'C:\\exports\\chart.png',
  editTarget: async (_workspaceId, resultId) => {
    await wait(120);
    const result = results.get(resultId);
    if (!result) throw fail('no-result', 'This result is no longer available.');
    if (result.columns.length > demoColumns.length) throw fail('not-editable', 'The result combines more than one table.');
    return {
      schema: 'sales',
      table: 'orders',
      keyColumns: [0],
      columns: result.columns.map((c) => (c.kind === 'binary' ? null : c.name)),
    };
  },
  previewEdits: async (_resultId, edits) =>
    edits.map((edit) => {
      const literal = (v: string | null) => (v === null ? 'NULL' : `'${v.replaceAll("'", "''")}'`);
      const name = (column: number) => `\`${demoColumns[column % demoColumns.length].name}\``;
      if (edit.type === 'delete') return `DELETE FROM \`sales\`.\`orders\` WHERE \`id\` = ${edit.row + 1} LIMIT 1;`;
      if (edit.type === 'insert') {
        return `INSERT INTO \`sales\`.\`orders\` (${edit.values.map((v) => name(v.column)).join(', ')}) VALUES (${edit.values.map((v) => literal(v.value)).join(', ')});`;
      }
      return `UPDATE \`sales\`.\`orders\` SET ${edit.changes.map((c) => `${name(c.column)} = ${literal(c.value)}`).join(', ')} WHERE \`id\` = ${edit.row + 1} LIMIT 1;`;
    }),
  applyEdits: async (_workspaceId, _tabId, _resultId, edits) => {
    await wait(300);
    return edits.length;
  },
  tableKeys: async (_workspaceId, _schema, table) => ({
    primaryKey: table === 'order_lines' ? ['order_id', 'line_no'] : [table === 'products' ? 'sku' : 'id'],
    indexes:
      table === 'orders'
        ? [
            { name: 'idx_customer', columns: ['customer_id'], unique: false, originalName: 'idx_customer', locked: false },
            { name: 'idx_status_placed', columns: ['status', 'placed_at'], unique: false, originalName: 'idx_status_placed', locked: false },
          ]
        : table === 'customers'
          ? [{ name: 'uq_email', columns: ['email'], unique: true, originalName: 'uq_email', locked: false }]
          : [],
    foreignKeys:
      table === 'orders'
        ? [{ name: 'fk_orders_customer', columns: ['customer_id'], referencedSchema: 'sales', referencedTable: 'customers', referencedColumns: ['id'], onUpdate: 'RESTRICT', onDelete: 'CASCADE' }]
        : [],
  }),
  tableSpec: async (_workspaceId, schema, table) => {
    await wait(150);
    const columns = schemas[schema]?.[table];
    if (!columns) return null;
    return {
      schema,
      name: table,
      columns: columns.map(([name, dataType, nullable]) => ({
        name,
        dataType,
        nullable,
        default: name === 'status' ? "'open'" : name === 'created_at' ? 'CURRENT_TIMESTAMP' : null,
        onUpdate: null,
        autoIncrement: name === 'id',
        comment: '',
        collation: null,
        originalName: name,
        locked: false,
      })),
      primaryKey: columns.filter(([, , , key]) => key === 'PRI').map(([name]) => name),
      indexes: columns
        .filter(([, , , key]) => key === 'MUL' || key === 'UNI')
        .map(([name, , , key]) => ({ name: `idx_${name}`, columns: [name], unique: key === 'UNI', originalName: `idx_${name}`, locked: false })),
      foreignKeys:
        table === 'orders'
          ? [{ name: 'fk_orders_customer', columns: ['customer_id'], referencedSchema: 'sales', referencedTable: 'customers', referencedColumns: ['id'], onUpdate: 'RESTRICT', onDelete: 'CASCADE', originalName: 'fk_orders_customer' }]
          : [],
      engine: 'InnoDB',
      collation: 'utf8mb4_0900_ai_ci',
      comment: table === 'orders' ? 'One row per customer order' : '',
    };
  },
  tableDdl: async (original, updated) => {
    if (updated.name.trim() === '') throw fail('other', 'Enter a name.');
    const incomplete = updated.foreignKeys.find((k) => k.columns.length === 0 || k.referencedTable === '' || k.referencedColumns.length !== k.columns.length);
    if (incomplete) throw fail('other', 'A foreign key needs its columns, the table it refers to, and one column there for each of its own.');
    const names = (list: string[]) => list.map((n) => `\`${n}\``).join(', ');
    const foreignKey = (k: (typeof updated.foreignKeys)[number]) =>
      `${k.name ? `CONSTRAINT \`${k.name}\` ` : ''}FOREIGN KEY (${names(k.columns)}) REFERENCES \`${k.referencedSchema || updated.schema}\`.\`${k.referencedTable}\` (${names(k.referencedColumns)})` +
      `${k.onDelete ? ` ON DELETE ${k.onDelete.toUpperCase()}` : ''}${k.onUpdate ? ` ON UPDATE ${k.onUpdate.toUpperCase()}` : ''}`;
    const blank = updated.columns.find((c) => c.name.trim() === '' || c.dataType.trim() === '');
    if (blank) throw fail('other', 'Every column needs a name and a type.');
    const define = (c: (typeof updated.columns)[number]) =>
      `\`${c.name}\` ${c.dataType}${c.nullable ? ' NULL' : ' NOT NULL'}${c.default ? ` DEFAULT ${c.default}` : ''}${c.autoIncrement ? ' AUTO_INCREMENT' : ''}`;
    if (!original) {
      const lines = updated.columns.map(define);
      if (updated.primaryKey.length) lines.push(`PRIMARY KEY (${updated.primaryKey.map((n) => `\`${n}\``).join(', ')})`);
      for (const i of updated.indexes) lines.push(`${i.unique ? 'UNIQUE ' : ''}INDEX \`${i.name}\` (${i.columns.map((n) => `\`${n}\``).join(', ')})`);
      for (const k of updated.foreignKeys) lines.push(foreignKey(k));
      return `CREATE TABLE \`${updated.schema}\`.\`${updated.name}\` (\n  ${lines.join(',\n  ')}\n);`;
    }
    const clauses: string[] = [];
    const drops: string[] = [];
    for (const before of original.foreignKeys) {
      const now = updated.foreignKeys.find((k) => k.originalName === before.name);
      if (!now || foreignKey(now) !== foreignKey(before)) drops.push(`DROP FOREIGN KEY \`${before.name}\``);
      if (now && foreignKey(now) !== foreignKey(before)) clauses.push(`ADD ${foreignKey(now)}`);
    }
    for (const k of updated.foreignKeys) if (k.originalName === null) clauses.push(`ADD ${foreignKey(k)}`);
    for (const before of original.columns) {
      if (!updated.columns.some((c) => c.originalName === before.name)) clauses.push(`DROP COLUMN \`${before.name}\``);
    }
    for (const c of updated.columns) {
      const before = original.columns.find((b) => b.name === c.originalName);
      if (!before) clauses.push(`ADD COLUMN ${define(c)}`);
      else if (before.name !== c.name) clauses.push(`CHANGE COLUMN \`${before.name}\` ${define(c)}`);
      else if (define(before) !== define(c)) clauses.push(`MODIFY COLUMN ${define(c)}`);
    }
    if (updated.name !== original.name) clauses.push(`RENAME TO \`${updated.schema}\`.\`${updated.name}\``);
    if (clauses.length === 0 && drops.length === 0) throw fail('other', 'Nothing has been changed.');
    const alter = (parts: string[]) => `ALTER TABLE \`${original.schema}\`.\`${original.name}\`\n  ${parts.join(',\n  ')};`;
    return [drops, clauses].filter((parts) => parts.length > 0).map(alter).join('\n\n');
  },
  objectDdl: async (action, kind, schema, name, newName) => {
    if (action === 'dropSchema') return `DROP SCHEMA \`${schema}\`;`;
    if (action === 'truncate') return `TRUNCATE TABLE \`${schema}\`.\`${name}\`;`;
    if (action === 'rename') {
      if (!newName?.trim()) throw fail('other', 'Enter a name.');
      return `RENAME TABLE \`${schema}\`.\`${name}\` TO \`${schema}\`.\`${newName.trim()}\`;`;
    }
    return `DROP ${kind.toUpperCase()} \`${schema}\`.\`${name}\`;`;
  },
  completionColumns: async (_workspaceId, schema) =>
    Object.entries(schemas[schema] ?? {}).flatMap(([table, columns]) => columns.map(([column]): [string, string] => [table, column])),
  historyList: async (search) => {
    const needle = search?.toLowerCase();
    return history.filter((h) => !needle || h.sql.toLowerCase().includes(needle));
  },
  historyClear: async () => {
    history.length = 0;
  },
  settingsGet: async () => {
    try {
      return { ...mockDefaults, ...JSON.parse(localStorage.getItem('sqltool.mock.settings') ?? '{}') };
    } catch {
      return { ...mockDefaults };
    }
  },
  settingsSet: async (settings) => {
    const saved = { ...settings, connectTimeoutSecs: Math.min(300, Math.max(3, settings.connectTimeoutSecs)) };
    localStorage.setItem('sqltool.mock.settings', JSON.stringify(saved));
    return saved;
  },
  selftestConfig: async () => null,
  selftestReport: async () => {},
};
