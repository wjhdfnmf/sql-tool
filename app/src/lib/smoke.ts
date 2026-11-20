// A short unattended check that the app runs at all on this system.
//
// Started with SQLTOOL_SMOKE set (development builds only), the app connects
// to the server built into it, runs a query through the real interface,
// copies, exports, and exits with a report. It needs no database, so it can
// run on every system the installers are built for; `tests/smoke.sh` runs it.
// The longer self-test in `selftest.ts` covers far more, against real servers.

import { backend } from './backend';
import { faults } from './faults.svelte';
import { loadLicences } from './licences';
import { decodeCells } from './rows';
import { app, type WorkspaceState } from './state.svelte';
import { asUiError } from './types';

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

export async function runSmoke(): Promise<void> {
  const steps: Step[] = [];
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

  let workspace: WorkspaceState | undefined;

  await step('the app started and knows its version and data folder', async () => {
    const info = await backend.appInfo();
    expect(/^\d+\.\d+\.\d+/.test(info.version), `version is ${info.version}`);
    expect(info.dataDir.length > 0, 'no data folder');
    return { version: info.version, platform: navigator.platform, agent: navigator.userAgent.slice(0, 120) };
  });

  await step('settings are written to the data folder and read back', async () => {
    const before = await backend.settingsGet();
    const size = before.editorTabSize === 4 ? 8 : 4;
    await backend.settingsSet({ ...before, editorTabSize: size });
    expect((await backend.settingsGet()).editorTabSize === size, 'the setting did not come back');
    await backend.settingsSet(before);
  });

  await step('connects to the built-in server over TLS', async () => {
    workspace = app.open(await backend.connectDemo());
    await sleep(600);
    return { server: workspace.info.info.version.raw, security: workspace.info.info.tlsVersion };
  });

  await step('the editor and the schema tree are drawn', async () => {
    expect(workspace, 'not connected');
    const scroller = document.querySelector('.cm-scroller');
    expect(scroller, 'the SQL editor did not render');
    const font = getComputedStyle(scroller).fontFamily;
    expect(/mono|consolas|cascadia|menlo/i.test(font), `the editor's styles were not applied (font is ${font})`);
    return { font: font.slice(0, 60), schemaNodes: document.querySelectorAll('.tree .node').length };
  });

  await step('a query runs and its rows are painted', async () => {
    expect(workspace, 'not connected');
    const tab = workspace.activeTab;
    await workspace.run(tab, 'SELECT fake_rows(2000, 5)', { kind: 'all' });
    for (let waited = 0; tab.running && waited < 15_000; waited += 100) await sleep(100);
    await sleep(500);
    expect(tab.results.length === 1, `expected one result; messages: ${tab.messages.map((m) => m.text).join(' | ')}`);
    const result = tab.results[0];
    expect(result.rows === 2000 && result.columns.length === 5, `got ${result.rows} rows of ${result.columns.length} columns`);
    const last = decodeCells(await backend.getRows(result.id, 1999, 1), 5);
    expect(last.rows === 1 && last.values.length === 5, 'the last row could not be read back');
    const canvas = document.querySelector('canvas');
    expect(canvas && canvas.width > 0 && canvas.height > 0, 'the result grid did not render');
    // The grid paints text, so a painted canvas is not one flat colour.
    const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, Math.min(canvas.height, 60)).data;
    let differing = 0;
    for (let i = 4; i < pixels.length; i += 4) {
      if (pixels[i] !== pixels[0] || pixels[i + 1] !== pixels[1] || pixels[i + 2] !== pixels[2]) differing++;
    }
    expect(differing > 50, 'the result grid is blank');
    return { grid: `${canvas.width}x${canvas.height}` };
  });

  await step('cells are copied as text', async () => {
    expect(workspace, 'not connected');
    const result = workspace.activeTab.results[0];
    expect(result, 'no result to copy from');
    const text = await backend.copyRange(result.id, { rowStart: 0, rowEnd: 1, colStart: 0, colEnd: 1 }, 'tsv', true, '');
    expect(text.split('\n').length >= 3 && text.includes('\t'), `copied: ${text.slice(0, 80)}`);
    // Onto the system's clipboard, the way Copy does it, and read back from
    // there: on Linux and macOS the web view's own clipboard refuses a write
    // that is not made inside a click.
    await backend.clipboardText(text);
    const back = await backend.selftestClipboard();
    expect(back === text, `the clipboard holds ${JSON.stringify(back.slice(0, 60))}, not what was copied`);
    return { clipboard: `${back.length} characters, read back the same` };
  });

  await step('a picture is copied', async () => {
    // What Copy image on a graph does, with a small picture drawn here.
    const drawing = document.createElement('canvas');
    drawing.width = 64;
    drawing.height = 40;
    const pen = drawing.getContext('2d')!;
    pen.fillStyle = '#1f5fbf';
    pen.fillRect(0, 0, 64, 40);
    const blob = await new Promise<Blob | null>((done) => drawing.toBlob(done, 'image/png'));
    expect(blob, 'no picture could be made');
    const png = new Uint8Array(await blob.arrayBuffer());
    await backend.clipboardImage(png);
    return { bytes: png.length };
  });

  await step('a result is exported to a file', async () => {
    expect(workspace, 'not connected');
    const result = workspace.activeTab.results[0];
    expect(result, 'no result to export');
    const saved = await backend.exportResult(result.id, 'csv', '', null);
    expect(saved && saved.rows === 2000, `exported ${saved?.rows} rows`);
    return { path: saved.path };
  });

  await step('the licences of what the app is built with are in it', async () => {
    const text = await loadLicences();
    expect(text.length > 20_000 && /Apache License/.test(text) && /MIT/.test(text), `only ${text.length} characters of licences`);
    return { characters: text.length };
  });

  await step('disconnects', async () => {
    expect(workspace, 'not connected');
    await app.close(workspace);
    expect(app.workspaces.length === 0, 'the connection is still listed');
  });

  await step('nothing was blocked and nothing went wrong unnoticed', async () => {
    expect(violations.length === 0, `the content security policy blocked something: ${violations.join('; ')}`);
    expect(faults.seen.length === 0, `errors that nothing handled: ${faults.seen.join('; ')}`);
  });

  const report = { ok: steps.every((s) => s.ok), passed: steps.filter((s) => s.ok).length, total: steps.length, steps };
  await backend.selftestReport(JSON.stringify(report, null, 2));
}
