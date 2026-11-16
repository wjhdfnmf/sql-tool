// A stand-in backend for running the UI in a plain browser (`npm run dev`).
// It fakes a server with a few schemas and synthetic results, in the same
// wire format the real backend uses, so every screen can be exercised
// without the desktop shell or a database.

import type { Backend } from './backend';
import type { ColumnInfo, HistoryEntry, Profile, RunMsg, ServerInfo, UiError, WorkspaceInfo } from './types';

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
    cleartextAuth: true,
    cleartextRiskAccepted: true,
    savePassword: true,
    color: null,
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
    cleartextAuth: true,
    cleartextRiskAccepted: false,
    savePassword: false,
    color: 'red',
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
    cleartextAuth: false,
    cleartextRiskAccepted: false,
    savePassword: true,
    color: 'green',
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

let nextId = 100;
const results = new Map<number, { columns: ColumnInfo[]; rows: number }>();
const cancelled = new Set<string>();

const demoColumns: ColumnInfo[] = [
  { name: 'id', kind: 'number' },
  { name: 'customer', kind: 'text' },
  { name: 'status', kind: 'text' },
  { name: 'total', kind: 'number' },
  { name: 'placed_at', kind: 'date' },
  { name: 'notes', kind: 'text' },
  { name: 'attributes', kind: 'json' },
  { name: 'receipt', kind: 'binary' },
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
  connect: async (profileId, password) => {
    await wait(400);
    const profile = profiles.find((p) => p.id === profileId);
    if (!profile) throw fail('no-profile', 'This connection no longer exists.');
    if (password === null && !profile.hasPassword) throw fail('password-required', 'A password is needed for this connection.');
    if (password === 'wrong') throw fail('access-denied', "access denied: Access denied for user 'ann'@'10.0.0.5' (using password: YES) (error 1045)");
    return {
      id: nextId++,
      name: profile.name,
      color: profile.color,
      host: `${profile.host}:${profile.port}`,
      user: profile.user,
      info: serverInfo(profile),
      security: profile.tls === 'disabled' ? 'none' : profile.tls === 'unverified' ? 'unverified' : 'verified',
      database: profile.database,
    } satisfies WorkspaceInfo;
  },
  connectDemo: async () => ({
    id: nextId++,
    name: 'Demo server',
    color: null,
    host: '127.0.0.1:53211',
    user: 'demo',
    info: serverInfo(null),
    security: 'verified',
    database: null,
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
        onEvent({ type: 'resultSet', index, resultId, columns });
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
    for (const id of ids) results.delete(id);
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
  scriptSave: async (path, suggestedName) => ({
    path: path ?? `C:\\scripts\\${suggestedName}`,
    name: path ? path.split('\\').pop()! : suggestedName,
    text: '',
  }),
  historyList: async (search) => {
    const needle = search?.toLowerCase();
    return history.filter((h) => !needle || h.sql.toLowerCase().includes(needle));
  },
  historyClear: async () => {
    history.length = 0;
  },
  selftestConfig: async () => null,
  selftestReport: async () => {},
};
