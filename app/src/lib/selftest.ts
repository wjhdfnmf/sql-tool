// Unattended end-to-end check of the desktop app's command layer.
//
// When the app is started with the SQLTOOL_SELFTEST_* environment variables
// set (development builds only), this drives the same backend calls the UI
// makes against a real server, writes a report, and the app exits. It checks
// the things unit tests cannot: argument and field naming across the
// JavaScript/Rust boundary, the binary row transport, and the keychain.

import { backend } from './backend';
import { app } from './state.svelte';
import { asUiError, serverLabel, type Profile, type RunMode, type RunMsg, type SelfTestConfig, type WorkspaceInfo } from './types';

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

/** Decodes the first `cells` cells of a row window into text (null for NULL). */
function decodeCells(u8: Uint8Array, cells: number): { rows: number; values: (string | null)[] } {
  const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
  const rows = dv.getUint32(0, true);
  const values: (string | null)[] = [];
  const decoder = new TextDecoder();
  let pos = 4;
  while (values.length < cells && pos < u8.length) {
    const b = u8[pos];
    if (b === 0xfb) {
      values.push(null);
      pos += 1;
    } else if (b < 251) {
      values.push(decoder.decode(u8.subarray(pos + 1, pos + 1 + b)));
      pos += 1 + b;
    } else {
      const l = dv.getUint16(pos + 1, true);
      values.push(decoder.decode(u8.subarray(pos + 3, pos + 3 + l)));
      pos += 3 + l;
    }
  }
  return { rows, values };
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
    expect(violations.length === 0, `the content security policy blocked something: ${violations.join('; ')}`);
    await backend.closeResults(tab.results.map((r) => r.id));
    await backend.closeTab(workspace.id, tab.id);
    return { editorFont: font.slice(0, 48), grid: `${canvas.width}x${canvas.height}`, schemaNodes };
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
