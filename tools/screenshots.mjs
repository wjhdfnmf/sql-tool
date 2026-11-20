// Takes the pictures of the app that the README shows.
//
// The interface runs in a plain browser against made-up data when it is not
// inside the desktop app (see app/src/lib/mock.ts), so a headless browser can
// be walked through it. Nothing here touches a database or the installed app.
//
//   cd app && npm run dev          in one terminal
//   node tools/screenshots.mjs     in another
//
// It needs Node 22 or newer and Edge, Chrome or Chromium; set SQLTOOL_BROWSER
// to the browser's path if it is not found. The pictures go to
// docs/screenshots.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'docs', 'screenshots');
const APP = 'http://localhost:5173/';
const PORT = 9333;
const WIDTH = 1400;
const HEIGHT = 880;

const browser = [
  process.env.SQLTOOL_BROWSER,
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
  '/usr/bin/google-chrome',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
].find((path) => path && existsSync(path));
if (!browser) {
  console.error('No Edge, Chrome or Chromium was found. Set SQLTOOL_BROWSER to its path.');
  process.exit(2);
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

try {
  await fetch(APP);
} catch {
  console.error(`Nothing answers at ${APP}. Start the interface first: cd app && npm run dev`);
  process.exit(2);
}

const profile = mkdtempSync(join(tmpdir(), 'sqltool-shots-'));
const child = spawn(
  browser,
  ['--headless=new', `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--hide-scrollbars', 'about:blank'],
  { stdio: 'ignore' },
);

let socket;
try {
  // The browser says where to talk to its one page.
  let page;
  for (let tries = 0; tries < 50 && !page; tries++) {
    await sleep(200);
    try {
      page = (await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json()).find((target) => target.type === 'page');
    } catch {
      // Not listening yet.
    }
  }
  if (!page) throw new Error('the browser did not start');

  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((opened, failed) => {
    socket.addEventListener('open', opened, { once: true });
    socket.addEventListener('error', () => failed(new Error('could not talk to the browser')), { once: true });
  });
  let nextId = 0;
  const waiting = new Map();
  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    const answer = waiting.get(message.id);
    if (!answer) return;
    waiting.delete(message.id);
    if (message.error) answer.failed(new Error(message.error.message));
    else answer.done(message.result);
  });
  const send = (method, params = {}) =>
    new Promise((done, failed) => {
      const id = ++nextId;
      waiting.set(id, { done, failed });
      socket.send(JSON.stringify({ id, method, params }));
    });

  /** Runs code in the page, with a few helpers for finding and pressing things. */
  const inPage = async (body) => {
    const expression = `(async () => {
      const wait = (ms) => new Promise((done) => setTimeout(done, ms));
      const button = (text) => [...document.querySelectorAll('button')].find((b) => b.textContent.trim().includes(text));
      const press = async (text, after = 400) => {
        const found = button(text);
        if (!found) throw new Error('no button says ' + text);
        found.click();
        await wait(after);
      };
      ${body}
    })()`;
    const result = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
  };

  const shoot = async (name) => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    writeFileSync(join(out, `${name}.png`), Buffer.from(data, 'base64'));
    console.log(`  docs/screenshots/${name}.png`);
  };

  mkdirSync(out, { recursive: true });
  // Twice the size in pixels, so the pictures stay sharp on a dense screen.
  await send('Emulation.setDeviceMetricsOverride', { width: WIDTH, height: HEIGHT, deviceScaleFactor: 2, mobile: false });
  await send('Page.navigate', { url: APP });
  await sleep(2000);

  await inPage(`
    await press('Open demo', 1500);
    document.querySelector('.cm-content').focus();
    document.execCommand('insertText', false, 'select * from plant.process');
    await wait(300);
    await press('Execute', 1800);
    // The completion list that typing brought up is not part of the picture.
    document.querySelector('.cm-content').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    document.querySelector('.grid .scroller')?.focus();
    await wait(400);
  `);
  await shoot('query');

  await inPage(`
    const column = (name) => [...document.querySelectorAll('.graph .column')].find((c) => c.querySelector('.column-name').textContent === name);
    const put = async (name, where) => {
      column(name).click();
      await wait(300);
      [...document.querySelectorAll('[role=menu] [role=menuitem]')].find((item) => item.textContent.trim() === where).click();
      await wait(700);
    };
    await press('Graph', 1200);
    [...document.querySelectorAll('.graph .kinds button')].find((b) => b.textContent === 'Control').click();
    await wait(500);
    await put('thickness', 'Put on the Y axis');
    await put('lot', 'Put on the X axis');
    await wait(1500);
  `);
  await shoot('graph');

  await send('Browser.close').catch(() => {});
} catch (e) {
  console.error(`screenshots: ${e instanceof Error ? e.message : e}`);
  process.exitCode = 1;
} finally {
  socket?.close();
  child.kill();
  await sleep(500);
  try {
    rmSync(profile, { recursive: true, force: true });
  } catch {
    // The browser may still hold a file for a moment; the folder is in the system's temporary place.
  }
}
