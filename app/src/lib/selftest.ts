// Unattended end-to-end check of the desktop app's command layer.
//
// When the app is started with the SQLTOOL_SELFTEST_* environment variables
// set (development builds only), this drives the same backend calls the UI
// makes against a real server, writes a report, and the app exits. It checks
// the things unit tests cannot: argument and field naming across the
// JavaScript/Rust boundary, the binary row transport, and the keychain.

import { backend } from './backend';
import { app } from './state.svelte';
import { decodeCells } from './rows';
import { asUiError, serverLabel, type Profile, type RowEdit, type RunMode, type RunMsg, type SelfTestConfig, type WorkspaceInfo } from './types';

interface Step {
  name: string;
  ok: boolean;
  ms: number;
  detail?: unknown;
  error?: string;
}

function expect(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function runSelfTest(config: SelfTestConfig): Promise<void> {
  const steps: Step[] = [];
  // Anything the content security policy blocks from here on is a defect.
  const violations: string[] = [];
  document.addEventListener('securitypolicyviolation', (e) => violations.push(`${e.violatedDirective} blocked ${e.blockedURI}`));
  const step = async (name: string, body: () => Promise<unknown>) => {
    const started = performance.now();
    try {
      const detail = await body();
      steps.push({ name, ok: true, ms: Math.round(performance.now() - started), detail });
    } catch (e) {
      steps.push({ name, ok: false, ms: Math.round(performance.now() - started), error: asUiError(e).message });
    }
  };

  let profile: Profile | undefined;
  let workspace: WorkspaceInfo | undefined;
  const tabId = crypto.randomUUID();

  const run = async (script: string, mode: RunMode, maxRows: number | null) => {
    expect(workspace, 'not connected');
    const messages: RunMsg[] = [];
    await backend.run(workspace.id, tabId, script, mode, maxRows, (m) => messages.push(m));
    return messages;
  };
  const ofType = <T extends RunMsg['type']>(messages: RunMsg[], type: T) =>
    messages.filter((m): m is Extract<RunMsg, { type: T }> => m.type === type);

  await step('app info', () => backend.appInfo());

  await step('save profile, password into the keychain', async () => {
    profile = await backend.profileSave(config.profile, config.password);
    expect(profile.id.length > 0, 'profile got no id');
    expect(profile.hasPassword, 'keychain did not keep the password');
    return { tls: profile.tls, cleartextAuth: profile.cleartextAuth };
  });

  await step('list profiles', async () => {
    const all = await backend.profilesList();
    const mine = all.find((p) => p.id === profile?.id);
    expect(mine, 'saved profile is not in the list');
    expect(mine.hasPassword, 'list does not report the stored password');
    expect(!('password' in mine), 'a password field came back from the backend');
    return { count: all.length };
  });

  await step('test connection (password from the keychain)', async () => {
    expect(profile, 'no profile');
    return serverLabel((await backend.connectionTest(profile, null)).version);
  });

  await step('connect (password from the keychain)', async () => {
    expect(profile, 'no profile');
    workspace = await backend.connect(profile.id, null);
    return {
      server: serverLabel(workspace.info.version),
      raw: workspace.info.version.raw,
      user: workspace.info.currentUser,
      tls: workspace.info.tlsVersion,
      security: workspace.security,
      database: workspace.database,
    };
  });

  await step('wrong password is refused', async () => {
    expect(profile, 'no profile');
    try {
      await backend.connectionTest(profile, 'definitely-not-the-password');
    } catch (e) {
      const error = asUiError(e);
      expect(error.kind === 'access-denied', `expected access-denied, got ${error.kind}: ${error.message}`);
      return error.kind;
    }
    throw new Error('a wrong password was accepted');
  });

  await step('fetch the certificate and connect with it pinned', async () => {
    const certificate = await backend.certificateProbe(config.profile.host, config.profile.port);
    expect(/^[0-9a-f]{64}$/.test(certificate.sha256) && certificate.subject.length > 0, `certificate was ${JSON.stringify(certificate)}`);
    const pinned = await backend.profileSave(
      { ...config.profile, name: 'Self-test pinned', tls: 'pinned', caFile: null, pinnedSha256: certificate.sha256 },
      config.password,
    );
    const wrong = await backend.profileSave(
      { ...config.profile, name: 'Self-test wrong pin', tls: 'pinned', caFile: null, pinnedSha256: 'ab'.repeat(32) },
      config.password,
    );
    try {
      const connected = await backend.connect(pinned.id, null);
      expect(connected.security === 'pinned' && connected.info.tlsVersion !== null, `connected as ${JSON.stringify(connected)}`);
      await backend.disconnect(connected.id);
      try {
        await backend.connect(wrong.id, null);
        throw new Error('a connection with the wrong pin was accepted');
      } catch (e) {
        expect(asUiError(e).kind === 'certificate-changed', `expected certificate-changed, got ${asUiError(e).kind}: ${asUiError(e).message}`);
      }
      return { subject: certificate.subject, issuer: certificate.issuer, selfSigned: certificate.selfSigned };
    } finally {
      await backend.profileDelete(pinned.id);
      await backend.profileDelete(wrong.id);
    }
  });

  await step('schema list', async () => {
    expect(workspace, 'not connected');
    const schemas = await backend.schemaList(workspace.id);
    expect(schemas.includes(config.schema), `schema ${config.schema} not listed`);
    return { count: schemas.length };
  });

  await step('schema objects', async () => {
    expect(workspace, 'not connected');
    const objects = await backend.schemaObjects(workspace.id, config.schema);
    expect(objects.tables.some((t) => t.name === config.table), `table ${config.table} not listed`);
    return {
      tables: objects.tables.filter((t) => !t.isView).length,
      views: objects.tables.filter((t) => t.isView).length,
      routines: objects.routines.length,
      triggers: objects.triggers.length,
      events: objects.events.length,
    };
  });

  await step('table columns', async () => {
    expect(workspace, 'not connected');
    const columns = await backend.tableColumns(workspace.id, config.schema, config.table);
    expect(columns.length > 0, 'no columns returned');
    expect(typeof columns[0].columnType === 'string' && columns[0].columnType.length > 0, 'column type missing');
    return columns.map((c) => `${c.name} ${c.columnType}${c.key ? ' ' + c.key : ''}`);
  });

  await step('object details, schema defaults, character sets', async () => {
    expect(workspace, 'not connected');
    const details = await backend.tableDetails(workspace.id, config.schema, config.table);
    expect(details && !details.isView && typeof details.engine === 'string', `details were ${JSON.stringify(details)}`);
    const defaults = await backend.schemaDefaults(workspace.id, config.schema);
    expect(defaults && defaults.charset.length > 0 && defaults.collation.length > 0, 'schema defaults missing');
    const charsets = await backend.charsets(workspace.id);
    const utf8mb4 = charsets.find((c) => c.name === 'utf8mb4');
    expect(utf8mb4 && utf8mb4.defaultCollation.length > 0, 'utf8mb4 not offered');
    const collations = await backend.collations(workspace.id, 'utf8mb4');
    expect(collations.includes(utf8mb4.defaultCollation), 'default collation not in the list');
    return { engine: details.engine, rowsEstimate: details.rowsEstimate, defaults, charsets: charsets.length, collations: collations.length };
  });

  await step('schema SQL, alter scripts and templates', async () => {
    expect(workspace, 'not connected');
    const create = await backend.schemaDdl(false, 'new shop', 'utf8mb4', null);
    expect(create === 'CREATE SCHEMA `new shop` DEFAULT CHARACTER SET utf8mb4;', `create was ${create}`);
    const alter = await backend.schemaDdl(true, config.schema, null, 'utf8mb4_bin');
    expect(alter.startsWith('ALTER SCHEMA'), `alter was ${alter}`);
    try {
      await backend.schemaDdl(false, 'x', 'utf8mb4; DROP SCHEMA y', null);
      throw new Error('a character set carrying SQL was accepted');
    } catch (e) {
      expect(asUiError(e).message.includes('not a valid'), `unexpected error: ${asUiError(e).message}`);
    }
    const script = await backend.alterScript(workspace.id, 'table', config.schema, config.table);
    expect(script.includes('ALTER TABLE') && script.includes('--   CREATE TABLE'), 'table alter script is malformed');
    const template = await backend.createTemplate('procedure', config.schema);
    expect(template.includes('DELIMITER $$') && template.includes('CREATE PROCEDURE'), 'procedure template is malformed');
    return { create, alter };
  });

  await step('explain the statement at the cursor', async () => {
    const script = 'SELECT 1;\nSELECT * FROM `' + config.table + '` WHERE id = 1;';
    const messages = await run(script, { kind: 'explainAt', offset: script.indexOf('WHERE') }, null);
    const failed = ofType(messages, 'failed')[0];
    expect(!failed, `explain failed: ${failed?.message}`);
    const started = ofType(messages, 'started');
    expect(started.length === 1 && started[0].sql.startsWith('EXPLAIN SELECT *'), `ran: ${started.map((m) => m.sql)}`);
    const set = ofType(messages, 'resultSet')[0];
    expect(set && set.columns.some((c) => c.name.toLowerCase() === 'select_type'), 'no execution plan came back');
    await backend.closeResults([set.resultId]);
    return set.columns.map((c) => c.name);
  });

  await step('risky statements are flagged for confirmation', async () => {
    const concerns = await backend.reviewScript('SELECT 1; UPDATE t SET a = 1; DROP TABLE t; DELETE FROM t WHERE id = 1', { kind: 'all' });
    expect(concerns.map((c) => c.index).join() === '1,2', `flagged ${JSON.stringify(concerns)}`);
    expect(concerns[0].reason.includes('WHERE') && concerns[1].sql.startsWith('DROP'), 'concern fields are wrong');
    return concerns.map((c) => c.reason);
  });

  await step('sort and filter a result', async () => {
    const messages = await run(`SELECT id, name FROM \`${config.table}\``, { kind: 'all' }, null);
    const set = ofType(messages, 'resultSet')[0];
    expect(set, `no result: ${ofType(messages, 'failed')[0]?.message}`);
    expect(set.sql.startsWith('SELECT id, name'), 'the result does not carry its statement');
    const total = ofType(messages, 'resultDone')[0].rows;
    const sorted = await backend.resultView(set.resultId, null, 1, true);
    expect(sorted === total, `sorting changed the row count: ${sorted} of ${total}`);
    const top = decodeCells(await backend.getRows(set.resultId, 0, 1), 2).values[1];
    const filtered = await backend.resultView(set.resultId, 'grace', null, false);
    expect(filtered === 1, `filter kept ${filtered} rows`);
    const only = decodeCells(await backend.getRows(set.resultId, 0, 5), 2);
    expect(only.rows === 1 && only.values[1] === 'Grace Hopper', `filtered rows were ${JSON.stringify(only)}`);
    expect((await backend.resultView(set.resultId, null, null, false)) === total, 'clearing the view did not restore the rows');
    await backend.closeResults([set.resultId]);
    return { total, firstWhenSortedByNameDescending: top };
  });

  await step('copy and total a selection of cells', async () => {
    expect(workspace, 'not connected');
    expect(workspace.profileId === profile?.id, `the workspace names connection ${workspace.profileId}`);
    const messages = await run(`SELECT id, name FROM \`${config.table}\` ORDER BY id`, { kind: 'all' }, null);
    const set = ofType(messages, 'resultSet')[0];
    expect(set, `no result: ${ofType(messages, 'failed')[0]?.message}`);
    const total = ofType(messages, 'resultDone')[0].rows;
    const all = { rowStart: 0, rowEnd: total - 1, colStart: 0, colEnd: 1 };
    const first = decodeCells(await backend.getRows(set.resultId, 0, 1), 2).values;

    const tsv = await backend.copyRange(set.resultId, all, 'tsv', true, '');
    const lines = tsv.split('\n');
    expect(lines.length === total + 1 && lines[0] === 'id\tname', `copied text began ${JSON.stringify(tsv.slice(0, 80))}`);
    expect(lines[1] === `${first[0]}\t${first[1]}`, `first copied row was ${JSON.stringify(lines[1])}`);
    const one = await backend.copyRange(set.resultId, { rowStart: 0, rowEnd: 0, colStart: 1, colEnd: 1 }, 'tsv', false, '');
    expect(one === first[1], `one cell copied as ${JSON.stringify(one)}`);
    const list = await backend.copyRange(set.resultId, { ...all, colEnd: 0 }, 'list', false, '');
    expect(list.split(', ').length === total && !list.includes("'"), `IN list was ${list}`);
    const insert = await backend.copyRange(set.resultId, { rowStart: 0, rowEnd: 0, colStart: 0, colEnd: 1 }, 'sql', false, 'people_copy');
    expect(insert.startsWith('INSERT INTO `people_copy` (`id`, `name`) VALUES (') && insert.endsWith(');'), `insert was ${insert}`);

    const stats = await backend.rangeStats(set.resultId, all);
    expect(stats.rows === total && stats.cells === total * 2 && stats.numbers === total, `stats were ${JSON.stringify(stats)}`);
    expect(stats.sum !== null && stats.min !== null && stats.max !== null && stats.min <= stats.max, `stats were ${JSON.stringify(stats)}`);
    await backend.closeResults([set.resultId]);
    return { list, sum: stats.sum, average: stats.average };
  });

  await step('a result stopped at the row limit says so, and only what it holds is "shown"', async () => {
    expect(workspace, 'not connected');
    const messages = await run(config.bigQuery, { kind: 'all' }, 1000);
    const done = ofType(messages, 'resultDone')[0];
    expect(done && done.truncated && done.rows === 1000, `the limited result was ${JSON.stringify(done)}`);
    await backend.closeResults([done.resultId]);
    // Exporting every row of a query is refused for anything that is not a plain read.
    try {
      await backend.exportQuery(workspace.id, `selftest-export-${workspace.id}`, 'DELETE FROM nothing_at_all', 'csv', '', () => {});
      throw new Error('a statement that changes data was accepted for a full export');
    } catch (e) {
      expect(asUiError(e).kind === 'not-exportable', `unexpected: ${asUiError(e).kind}: ${asUiError(e).message}`);
    }
    return { rowsHeld: done.rows };
  });

  await step('a result loaded in full is exported in full, from what is held', async () => {
    expect(workspace, 'not connected');
    // No row limit: every row is fetched and kept, as when scrolling through millions of rows.
    const started = performance.now();
    const messages = await run(config.exportQuery, { kind: 'all' }, null);
    const done = ofType(messages, 'resultDone')[0];
    expect(done && !done.truncated, `the full load ended as ${JSON.stringify(done ?? ofType(messages, 'failed')[0])}`);
    const loadedMs = Math.round(performance.now() - started);
    // The last row is really there.
    const last = decodeCells(await backend.getRows(done.resultId, done.rows - 1, 1), 1);
    expect(last.rows === 1, `the last row could not be read back: ${JSON.stringify(last)}`);

    const exported: Record<string, number> = {};
    for (const format of ['csv', 'sql'] as const) {
      const saved = await backend.exportResult(done.resultId, format, 'copy');
      expect(saved, 'the export was cancelled');
      expect(saved.rows === done.rows, `${format}: exported ${saved.rows} of the ${done.rows} rows held`);
      expect(!saved.truncated && !saved.filtered, `${format}: the export was marked partial: ${JSON.stringify(saved)}`);
      exported[format] = saved.rows;
    }
    // Sorted and filtered, the export follows what is shown.
    const visible = await backend.resultView(done.resultId, '77', 0, true);
    const filtered = await backend.exportResult(done.resultId, 'tsv', '');
    expect(filtered && filtered.rows === visible && filtered.filtered, `filtered export wrote ${filtered?.rows} of ${visible} visible rows`);
    await backend.closeResults([done.resultId]);
    return { rows: done.rows, loadedMs, exported, filteredRows: visible };
  });

  await step('edit rows through a result, in one transaction', async () => {
    expect(workspace, 'not connected');
    const setup = await run(
      "DROP TABLE IF EXISTS selftest_edit; CREATE TABLE selftest_edit (id INT PRIMARY KEY, name VARCHAR(50) NULL); INSERT INTO selftest_edit VALUES (1, 'one'), (2, 'two')",
      { kind: 'all' },
      null,
    );
    expect(!ofType(setup, 'failed')[0], `setup failed: ${ofType(setup, 'failed')[0]?.message}`);
    const read = await run('SELECT id, name FROM selftest_edit ORDER BY id', { kind: 'all' }, null);
    const set = ofType(read, 'resultSet')[0];
    expect(set.columns[0].primaryKey && set.columns[0].source?.table === 'selftest_edit', `column source was ${JSON.stringify(set.columns[0])}`);
    const target = await backend.editTarget(workspace.id, set.resultId);
    expect(target.table === 'selftest_edit' && target.keyColumns.join() === '0' && target.columns[1] === 'name', `target was ${JSON.stringify(target)}`);
    const edits: RowEdit[] = [
      { type: 'update', row: 0, changes: [{ column: 1, value: "it's uno" }] },
      { type: 'delete', row: 1 },
      { type: 'insert', values: [{ column: 0, value: '3' }, { column: 1, value: null }] },
    ];
    const preview = await backend.previewEdits(set.resultId, edits);
    expect(preview.length === 3 && preview[0].includes("SET `name` = 'it''s uno' WHERE `id` = 1"), `preview was ${JSON.stringify(preview)}`);
    const applied = await backend.applyEdits(workspace.id, tabId, set.resultId, edits);
    expect(applied === 3, `applied ${applied} statements`);
    const after = await run('SELECT id, name FROM selftest_edit ORDER BY id', { kind: 'all' }, null);
    const rows = decodeCells(await backend.getRows(ofType(after, 'resultSet')[0].resultId, 0, 10), 4);
    expect(rows.rows === 2 && rows.values.join('|') === "1|it's uno|3|", `rows after editing: ${JSON.stringify(rows)}`);
    await run('DROP TABLE selftest_edit', { kind: 'all' }, null);
    await backend.closeResults([set.resultId, ofType(after, 'resultSet')[0].resultId]);
    return preview;
  });

  await step('table designer, keys, and drop/rename SQL', async () => {
    expect(workspace, 'not connected');
    const spec = await backend.tableSpec(workspace.id, config.schema, config.table);
    expect(spec && spec.columns.length > 0 && spec.primaryKey.join() === 'id', `spec was ${JSON.stringify(spec)?.slice(0, 200)}`);
    expect(spec.columns[0].autoIncrement && typeof spec.columns[0].dataType === 'string', 'column fields are misnamed');
    const create = await backend.tableDdl(null, { ...spec, name: 'copy_of_table' });
    expect(create.startsWith('CREATE TABLE') && create.includes('PRIMARY KEY (`id`)'), `create was ${create.slice(0, 120)}`);
    try {
      await backend.tableDdl(spec, spec);
      throw new Error('an unchanged table produced an ALTER');
    } catch (e) {
      expect(asUiError(e).message.includes('Nothing has been changed'), `unexpected: ${asUiError(e).message}`);
    }
    const changed = structuredClone(spec);
    changed.columns.push({ ...changed.columns[0], name: 'extra', dataType: 'INT', autoIncrement: false, nullable: true, originalName: null });
    const alter = await backend.tableDdl(spec, changed);
    expect(alter.includes('ADD COLUMN `extra` INT NULL AFTER'), `alter was ${alter}`);
    expect(Array.isArray(spec.foreignKeys), 'the table carries no list of foreign keys');
    const withKey = structuredClone(spec);
    withKey.foreignKeys.push({
      name: 'fk_self', columns: ['id'], referencedSchema: '', referencedTable: config.table, referencedColumns: ['id'],
      onUpdate: '', onDelete: 'CASCADE', originalName: null,
    });
    const keyed = await backend.tableDdl(spec, withKey);
    expect(keyed.includes('ADD CONSTRAINT `fk_self` FOREIGN KEY (`id`) REFERENCES') && keyed.includes('ON DELETE CASCADE'), `foreign key SQL was ${keyed}`);
    const keys = await backend.tableKeys(workspace.id, config.schema, config.table);
    expect(keys && keys.primaryKey.join() === 'id' && Array.isArray(keys.foreignKeys), `keys were ${JSON.stringify(keys)}`);
    const drop = await backend.objectDdl('drop', 'table', config.schema, 'x', null);
    expect(drop === `DROP TABLE \`${config.schema}\`.\`x\`;`, `drop was ${drop}`);
    const rename = await backend.objectDdl('rename', 'table', config.schema, 'x', 'y');
    expect(rename.includes('RENAME TABLE') && rename.includes('`y`'), `rename was ${rename}`);
    const names = await backend.completionColumns(workspace.id, config.schema);
    expect(names.some(([t, c]) => t === config.table && c === 'email'), 'completion names are missing');
    return { alter, indexes: keys.indexes.map((i) => i.name), completionNames: names.length };
  });

  await step('a read-only connection refuses changes', async () => {
    const saved = await backend.profileSave({ ...config.profile, name: 'Self-test read-only', readOnly: true }, config.password);
    try {
      const readOnly = await backend.connect(saved.id, null);
      expect(readOnly.readOnly, 'the connection does not report itself read-only');
      const roTab = crypto.randomUUID();
      const collect = async (sql: string) => {
        const messages: RunMsg[] = [];
        await backend.run(readOnly.id, roTab, sql, { kind: 'all' }, null, (m) => messages.push(m));
        return messages;
      };
      const write = await collect('CREATE TABLE selftest_must_not_exist (id INT)');
      const refused = ofType(write, 'failed')[0];
      expect(refused && refused.message.includes('read-only'), `the change was not refused: ${JSON.stringify(write)}`);
      const read = await collect('SELECT 1');
      const set = ofType(read, 'resultSet')[0];
      expect(set, 'a plain read was refused');
      try {
        await backend.editTarget(readOnly.id, set.resultId);
        throw new Error('a read-only connection offered editing');
      } catch (e) {
        expect(asUiError(e).kind === 'not-editable', `unexpected: ${asUiError(e).message}`);
      }
      await backend.closeResults([set.resultId]);
      await backend.disconnect(readOnly.id);
      return refused.message;
    } finally {
      await backend.profileDelete(saved.id);
    }
  });

  await step('run a script of two statements', async () => {
    const messages = await run("SELECT 1 AS one, 'café' AS word;\nSELECT 2 AS two, NULL AS nothing, 3.5 AS amount;", { kind: 'all' }, null);
    const sets = ofType(messages, 'resultSet');
    expect(sets.length === 2, `expected 2 result sets, got ${sets.length}`);
    expect(sets[0].columns.map((c) => c.name).join() === 'one,word', 'first result has the wrong columns');
    expect(sets[0].columns[0].kind === 'number' && sets[0].columns[1].kind === 'text', 'column kinds are wrong');
    const first = decodeCells(await backend.getRows(sets[0].resultId, 0, 10), 2);
    expect(first.rows === 1 && first.values[0] === '1' && first.values[1] === 'café', `first row decoded as ${JSON.stringify(first)}`);
    const second = decodeCells(await backend.getRows(sets[1].resultId, 0, 10), 3);
    expect(second.values[1] === null, 'NULL did not arrive as NULL');
    const cell = await backend.getCell(sets[1].resultId, 0, 2);
    expect(cell.text === '3.5', `cell value was ${cell.text}`);
    await backend.closeResults(sets.map((s) => s.resultId));
    return { finished: ofType(messages, 'finished')[0], second: second.values };
  });

  await step('run only the statement at the cursor', async () => {
    const script = 'SELECT 1 AS a;\nSELECT 2 AS b, 3 AS c;\nSELECT 4 AS d;';
    const messages = await run(script, { kind: 'at', offset: script.indexOf('2 AS b') }, null);
    const sets = ofType(messages, 'resultSet');
    expect(sets.length === 1, `expected 1 result set, got ${sets.length}`);
    expect(sets[0].columns.map((c) => c.name).join() === 'b,c', `ran the wrong statement: ${sets[0].columns.map((c) => c.name)}`);
    await backend.closeResults([sets[0].resultId]);
    return ofType(messages, 'started').map((m) => m.sql);
  });

  await step('large query stops at the row limit', async () => {
    const messages = await run(config.bigQuery, { kind: 'all' }, 1000);
    const failed = ofType(messages, 'failed')[0];
    expect(!failed, `query failed: ${failed?.message}`);
    const done = ofType(messages, 'resultDone')[0];
    expect(done, 'no result');
    expect(done.rows === 1000 && done.truncated, `expected 1000 truncated rows, got ${done.rows} truncated=${done.truncated}`);
    const tail = decodeCells(await backend.getRows(done.resultId, 990, 20), 1);
    expect(tail.rows === 10, `expected the last 10 rows, got ${tail.rows}`);
    const last = await backend.getCell(done.resultId, 999, 0);
    await backend.closeResults([done.resultId]);
    return { rows: done.rows, elapsedMs: done.elapsedMs, row991: tail.values[0], row1000: last.text };
  });

  await step('session still works after hitting the limit', async () => {
    const messages = await run('SELECT 7 AS seven', { kind: 'all' }, null);
    const set = ofType(messages, 'resultSet')[0];
    expect(set, `no result: ${ofType(messages, 'failed')[0]?.message}`);
    const row = decodeCells(await backend.getRows(set.resultId, 0, 1), 1);
    expect(row.values[0] === '7', `got ${row.values[0]}`);
    await backend.closeResults([set.resultId]);
  });

  await step('an error stops the script', async () => {
    const messages = await run('SELECT 1; SELEC 2; SELECT 3', { kind: 'all' }, null);
    const finished = ofType(messages, 'finished')[0];
    expect(finished.failed && finished.ran === 2 && finished.total === 3, `finished was ${JSON.stringify(finished)}`);
    await backend.closeResults(ofType(messages, 'resultSet').map((s) => s.resultId));
    return ofType(messages, 'failed')[0].message;
  });

  await step('cancel a running statement', async () => {
    expect(workspace, 'not connected');
    const started = performance.now();
    const running = run('SELECT SLEEP(20)', { kind: 'all' }, null);
    await new Promise((resolve) => setTimeout(resolve, 400));
    await backend.cancel(workspace.id, tabId);
    const messages = await running;
    const elapsed = Math.round(performance.now() - started);
    expect(elapsed < 8000, `cancel took ${elapsed} ms`);
    await backend.closeResults(ofType(messages, 'resultSet').map((s) => s.resultId));
    const after = await run('SELECT 8 AS eight', { kind: 'all' }, null);
    expect(ofType(after, 'resultSet').length === 1, 'session unusable after cancel');
    await backend.closeResults(ofType(after, 'resultSet').map((s) => s.resultId));
    return { elapsedMs: elapsed };
  });

  await step('settings are stored, and the query timeout stops a long statement', async () => {
    expect(workspace, 'not connected');
    const before = await backend.settingsGet();
    expect(before.theme === 'system' && before.connectTimeoutSecs === 15 && before.queryTimeoutSecs === 0, `fresh settings were ${JSON.stringify(before)}`);
    const saved = await backend.settingsSet({ ...before, theme: 'midnight', queryTimeoutSecs: 1, connectTimeoutSecs: 100000 });
    expect(saved.theme === 'midnight' && saved.connectTimeoutSecs === 300, `out-of-range values were not corrected: ${JSON.stringify(saved)}`);
    const started = performance.now();
    const messages = await run(config.bigQuery, { kind: 'all' }, null);
    const elapsed = Math.round(performance.now() - started);
    const failure = ofType(messages, 'failed')[0];
    await backend.closeResults(ofType(messages, 'resultSet').map((s) => s.resultId));
    await backend.settingsSet(before);
    expect(failure && failure.message.includes('query timeout in Settings'), `the statement was not stopped by the timeout: ${failure?.message}`);
    expect(elapsed < 15000, `stopping took ${elapsed} ms`);
    const after = await run('SELECT 9 AS nine', { kind: 'all' }, null);
    expect(ofType(after, 'resultSet').length === 1, 'the session is unusable after a timeout');
    await backend.closeResults(ofType(after, 'resultSet').map((s) => s.resultId));
    expect((await backend.settingsGet()).queryTimeoutSecs === 0, 'the settings were not put back');
    return { elapsedMs: elapsed, message: failure.message };
  });

  await step('a connection the server dropped is reopened on the next run', async () => {
    expect(workspace, 'not connected');
    const asked = await run('SELECT CONNECTION_ID() AS id', { kind: 'all' }, null);
    const set = ofType(asked, 'resultSet')[0];
    expect(set, `no connection id: ${ofType(asked, 'failed')[0]?.message}`);
    const id = decodeCells(await backend.getRows(set.resultId, 0, 1), 1).values[0];
    await backend.closeResults([set.resultId]);

    // Another connection kills it, which is what a server's idle timeout amounts to.
    const killer = `selftest-killer-${workspace.id}`;
    const killed: RunMsg[] = [];
    await backend.run(workspace.id, killer, `KILL ${id}`, { kind: 'all' }, null, (m) => killed.push(m));
    expect(!ofType(killed, 'failed')[0], `could not kill the connection: ${ofType(killed, 'failed')[0]?.message}`);
    await backend.closeTab(workspace.id, killer);
    await sleep(400);

    // The statement that finds the connection gone fails and says what happens next.
    const lost = await run('SELECT 1', { kind: 'all' }, null);
    const failure = ofType(lost, 'failed')[0];
    expect(failure && failure.message.includes('will be reopened'), `losing the connection was reported as: ${failure?.message}`);

    // The next run reopens it before sending anything, and says so.
    const again = await run('SELECT 2 AS two', { kind: 'all' }, null);
    expect(ofType(again, 'reconnected').length === 1, `no reconnect was reported: ${JSON.stringify(again.map((m) => m.type))}`);
    const result = ofType(again, 'resultSet')[0];
    expect(result, `the statement after reconnecting failed: ${ofType(again, 'failed')[0]?.message}`);
    const two = decodeCells(await backend.getRows(result.resultId, 0, 1), 1).values[0];
    await backend.closeResults([result.resultId]);
    expect(two === '2', `after reconnecting the result was ${two}`);

    // Cancelling still reaches the new connection.
    const started = performance.now();
    const running = run('SELECT SLEEP(20)', { kind: 'all' }, null);
    await sleep(400);
    await backend.cancel(workspace.id, tabId);
    const stopped = await running;
    await backend.closeResults(ofType(stopped, 'resultSet').map((s) => s.resultId));
    const elapsed = Math.round(performance.now() - started);
    expect(elapsed < 8000, `cancel after reconnecting took ${elapsed} ms`);
    return { lostMessage: failure.message.slice(0, 80), cancelMs: elapsed };
  });

  await step('the default schema is the connection\'s: new and other tabs adopt it', async () => {
    expect(workspace, 'not connected');
    const schemaOf = async (tab: string) => {
      const messages: RunMsg[] = [];
      await backend.run(workspace!.id, tab, 'SELECT DATABASE() AS db', { kind: 'all' }, null, (m) => messages.push(m));
      const set = ofType(messages, 'resultSet')[0];
      expect(set, `no result in ${tab}: ${ofType(messages, 'failed')[0]?.message}`);
      const value = decodeCells(await backend.getRows(set.resultId, 0, 1), 1).values[0];
      await backend.closeResults([set.resultId]);
      return value;
    };
    await backend.setDefaultSchema(workspace.id, 'information_schema');
    // A tab that has never run anything: its connection is opened now.
    const fresh = `selftest-fresh-${workspace.id}`;
    const inNewTab = await schemaOf(fresh);
    // A tab that already had the old default.
    const inOldTab = await schemaOf(tabId);
    await backend.closeTab(workspace.id, fresh);
    await backend.setDefaultSchema(workspace.id, config.schema);
    const back = await schemaOf(tabId);
    expect(inNewTab === 'information_schema', `a new tab had default schema ${inNewTab}`);
    expect(inOldTab === 'information_schema', `an existing tab kept default schema ${inOldTab}`);
    expect(back === config.schema, `after switching back the tab had ${back}`);
    try {
      await backend.setDefaultSchema(workspace.id, 'no_such_schema_selftest');
      throw new Error('a schema that does not exist was accepted');
    } catch (e) {
      expect(asUiError(e).message.includes('no_such_schema_selftest') || asUiError(e).kind === 'server' || asUiError(e).kind === 'access-denied', `unexpected: ${asUiError(e).message}`);
    }
    return { inNewTab, inOldTab, back };
  });

  await step('default schema follows USE', async () => {
    const messages = await run(`USE \`${config.schema}\``, { kind: 'all' }, null);
    const changed = ofType(messages, 'database')[0];
    expect(changed?.name === config.schema, `database event was ${JSON.stringify(changed)}`);
    const done = ofType(messages, 'done')[0];
    expect(done && typeof done.affectedRows === 'number', 'no completion message for USE');
  });

  await step('history recorded the statements', async () => {
    const found = await backend.historyList('SLEEP', 10);
    expect(found.length > 0, 'the SLEEP statement is not in the history');
    expect(typeof found[0].atMs === 'number' && typeof found[0].durationMs === 'number', 'history fields are misnamed');
    return { entries: (await backend.historyList(null, 100)).length };
  });

  await step('the screens render in the desktop webview', async () => {
    expect(workspace, 'not connected');
    const state = app.open(workspace);
    await sleep(400);
    const scroller = document.querySelector('.cm-scroller');
    expect(scroller, 'the SQL editor did not render');
    const font = getComputedStyle(scroller).fontFamily;
    expect(/mono|consolas|cascadia|menlo/i.test(font), `the editor's styles were not applied (font is ${font})`);
    const schemaNodes = document.querySelectorAll('.tree .node').length;
    expect(schemaNodes > 0, 'the schema tree is empty');

    await state.run(state.activeTab, "SELECT 1 AS one, 'two' AS two", { kind: 'all' });
    await sleep(400);
    const tab = state.activeTab;
    expect(tab.results.length === 1, `expected one result tab; messages: ${tab.messages.map((m) => m.text).join(' | ')}`);
    const canvas = document.querySelector('canvas');
    expect(canvas && canvas.width > 0 && canvas.height > 0, 'the result grid did not render');
    // The grid paints text, so a painted canvas is not one flat colour.
    const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, Math.min(canvas.height, 60)).data;
    let differing = 0;
    for (let i = 4; i < pixels.length; i += 4) {
      if (pixels[i] !== pixels[0] || pixels[i + 1] !== pixels[1] || pixels[i + 2] !== pixels[2]) differing++;
    }
    expect(differing > 50, 'the result grid is blank');
    // Selecting everything with the keyboard reports the selection and its totals under the grid.
    const input = document.querySelector<HTMLElement>('.grid .scroller');
    expect(input, 'the grid has no input layer');
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, bubbles: true }));
    await sleep(600);
    const footer = document.querySelector('footer.status .status-items:not([hidden])')?.textContent ?? '';
    expect(/1\s+row\s+×\s+2\s+columns/.test(footer) && /Sum\s+1\b/.test(footer), `selecting everything showed: ${footer.replace(/\s+/g, ' ').trim().slice(0, 200)}`);

    // The table designer opens in a tab, creates a table, then alters it.
    await state.execute(`DROP TABLE IF EXISTS \`${config.schema}\`.selftest_designed`);
    const designTab = state.openDesign({ kind: 'table', schema: config.schema, table: null });
    await sleep(900);
    const designer = document.querySelector<HTMLElement>('.design-area:not([hidden]) .designer');
    expect(designer, 'the table designer did not open in a tab');
    const nameInput = designer.querySelector<HTMLInputElement>('.head input');
    expect(nameInput, 'the table designer has no name field');
    nameInput.value = 'selftest_designed';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    await sleep(500);
    const press = (label: string) =>
      [...designer.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === label)?.click();
    const shown = () => designer.querySelector('pre')?.textContent ?? '';
    expect(shown().startsWith('CREATE TABLE') && shown().includes('selftest_designed'), `the designer's SQL was ${shown()}`);
    press('Create Table');
    await sleep(1500);
    expect(
      designTab.title === 'selftest_designed (table)' && !designTab.designDirty,
      `after creating, the tab was "${designTab.title}": ${designer.querySelector('.actions')?.textContent?.trim()}`,
    );
    press('+ created_at');
    await sleep(500);
    expect(shown().includes('ADD COLUMN `created_at` TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP AFTER `id`'), `the alter SQL was ${shown()}`);
    press('Apply Changes');
    await sleep(1500);
    const designed = await backend.tableSpec(workspace.id, config.schema, 'selftest_designed');
    const designedColumns = designed?.columns.map((c) => c.name).join();
    expect(designedColumns === 'id,created_at', `the designed table has columns ${designedColumns}`);
    await state.execute(`DROP TABLE \`${config.schema}\`.selftest_designed`);
    state.closeTab(designTab);

    // The schema editor is a tab too.
    const schemaTab = state.openDesign({ kind: 'schema', schema: config.schema });
    await sleep(800);
    const schemaHeading = document.querySelector('.design-area:not([hidden]) .editor h2')?.textContent ?? '';
    expect(schemaHeading.includes(config.schema), `the schema editor showed "${schemaHeading}"`);
    state.closeTab(schemaTab);

    expect(violations.length === 0, `the content security policy blocked something: ${violations.join('; ')}`);
    await backend.closeResults(tab.results.map((r) => r.id));
    await backend.closeTab(workspace.id, tab.id);
    return { editorFont: font.slice(0, 48), grid: `${canvas.width}x${canvas.height}`, schemaNodes, designedColumns };
  });

  await step('disconnect and remove the profile', async () => {
    expect(workspace && profile, 'nothing to clean up');
    await backend.closeTab(workspace.id, tabId);
    await backend.disconnect(workspace.id);
    await backend.profileDelete(profile.id);
    const all = await backend.profilesList();
    expect(!all.some((p) => p.id === profile?.id), 'profile was not deleted');
    try {
      await backend.schemaList(workspace.id);
    } catch (e) {
      return asUiError(e).kind;
    }
    throw new Error('the workspace still answered after disconnect');
  });

  const report = { ok: steps.every((s) => s.ok), passed: steps.filter((s) => s.ok).length, total: steps.length, steps };
  await backend.selftestReport(JSON.stringify(report, null, 2));
}
