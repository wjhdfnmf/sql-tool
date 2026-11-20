// Collects the licences of everything the app is built with into one text
// file, which the app shows under Help. Most of those licences ask that their
// text goes wherever the software goes, and an installer carries no
// repository along; this is how the text gets into it.
//
// Run by `npm run dev` and `npm run build` in `app`, before the interface is
// built. It reads the npm packages that ship in the app (from
// package-lock.json and node_modules) and the Rust crates the desktop shell
// is linked from (from `cargo metadata`), for every system at once, so the
// same file is right for the Windows, Linux and macOS installers.
//
//   node tools/licences.mjs            a file is always written; if the
//                                      crates cannot be listed it says so
//   node tools/licences.mjs --strict   fails instead: what a build uses

import { execFileSync } from 'node:child_process';
import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'app', 'src', 'generated', 'licences.txt');
const strict = process.argv.includes('--strict');

const LICENCE_FILE = /^(licen[cs]e|copying|notice|unlicense|copyright)([-._ ].*)?$/i;
const RULE = '='.repeat(80);
const LINE = '-'.repeat(80);

/** The licence, copying and notice files in a package's folder. */
function textsIn(dir) {
  let names;
  try {
    names = readdirSync(dir);
  } catch {
    return [];
  }
  const texts = [];
  for (const name of names.filter((n) => LICENCE_FILE.test(n)).sort()) {
    const path = join(dir, name);
    try {
      const stat = statSync(path);
      if (!stat.isFile() || stat.size > 400_000) continue;
      const text = readFileSync(path, 'utf8').replace(/\r\n?/g, '\n').replace(/[ \t]+$/gm, '').trim();
      if (text) texts.push(text);
    } catch {
      // Unreadable: treated as not there.
    }
  }
  return texts;
}

/** The npm packages that are compiled into the app's window: everything in the lock file that is not for development only. */
function npmPackages() {
  const lock = JSON.parse(readFileSync(join(root, 'app', 'package-lock.json'), 'utf8'));
  const found = [];
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (path === '' || entry.dev || !path.includes('node_modules/')) continue;
    const dir = join(root, 'app', path);
    let manifest;
    try {
      manifest = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
    } catch {
      continue; // An optional package for another system: not installed here, and not in the app.
    }
    const licence = typeof manifest.license === 'string' ? manifest.license : (entry.license ?? 'see its licence text');
    const home = typeof manifest.repository === 'string' ? manifest.repository : (manifest.repository?.url ?? manifest.homepage ?? '');
    found.push({ name: manifest.name, version: manifest.version, licence, home: tidy(home), texts: textsIn(dir) });
  }
  return found;
}

function tidy(url) {
  return String(url ?? '')
    .replace(/^git\+/, '')
    .replace(/\.git$/, '');
}

/** The crates the desktop app is linked from, on any system: its dependencies, and theirs, leaving out what is only used to build or test. */
function crates() {
  const metadata = JSON.parse(
    execFileSync('cargo', ['metadata', '--format-version', '1', '--locked'], { cwd: root, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] }),
  );
  const byId = new Map(metadata.packages.map((p) => [p.id, p]));
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const app = metadata.packages.find((p) => p.name === 'sqltool-app');
  if (!app) throw new Error('the workspace has no sqltool-app package');
  const ours = new Set(metadata.workspace_members);
  const seen = new Set([app.id]);
  const queue = [app.id];
  while (queue.length > 0) {
    for (const dep of nodes.get(queue.pop())?.deps ?? []) {
      // `kind` is null for an ordinary dependency, "build" or "dev" for the others.
      if (seen.has(dep.pkg) || !dep.dep_kinds.some((k) => k.kind === null)) continue;
      seen.add(dep.pkg);
      queue.push(dep.pkg);
    }
  }
  const found = [];
  for (const id of seen) {
    if (ours.has(id)) continue; // This project's own crates: under its own licence.
    const p = byId.get(id);
    const dir = dirname(p.manifest_path);
    const texts = textsIn(dir);
    if (p.license_file) {
      try {
        const text = readFileSync(join(dir, p.license_file), 'utf8').replace(/\r\n?/g, '\n').trim();
        if (text && !texts.includes(text)) texts.push(text);
      } catch {
        // Named but not there.
      }
    }
    found.push({ name: p.name, version: p.version, licence: p.license ?? 'see its licence text', home: tidy(p.repository ?? p.homepage ?? ''), texts });
  }
  return found;
}

const byName = (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version);
const label = (p) => `${p.name} ${p.version}`;

function listing(title, packages) {
  const lines = [title, ''];
  for (const p of [...packages].sort(byName)) lines.push(`  ${label(p)}  -  ${p.licence}${p.home ? `  -  ${p.home}` : ''}`);
  return lines.join('\n');
}

let ui = [];
let shell = [];
let problem = null;
try {
  ui = npmPackages();
  shell = crates();
} catch (e) {
  problem = e instanceof Error ? e.message.split('\n')[0] : String(e);
  if (strict) {
    console.error(`licences: ${problem}`);
    process.exit(1);
  }
}

// The same text is printed once, with everything that uses it named above it.
const users = new Map();
const bare = [];
for (const p of [...ui, ...shell]) {
  if (p.texts.length === 0) bare.push(p);
  for (const text of p.texts) {
    const key = text.replace(/\s+/g, ' ');
    if (!users.has(key)) users.set(key, { text, packages: [] });
    users.get(key).packages.push(p);
  }
}

const parts = [
  'SQL Tool is free software under the Apache License, Version 2.0.',
  '',
  'It is built with the open-source software listed here. Each piece stays under',
  'its own licence; the texts of those licences follow the list.',
  '',
];
if (problem) {
  parts.push(`This list is incomplete: it could not be put together when this copy was built (${problem}).`, '');
}
parts.push(RULE, 'WHAT THE APP IS BUILT WITH', RULE, '');
parts.push(listing(`In the window (${ui.length} npm packages)`, ui), '');
parts.push(listing(`In the desktop shell and core (${shell.length} Rust crates, over Windows, Linux and macOS)`, shell), '');
if (bare.length > 0) {
  parts.push(
    'These come without a licence file of their own. Each is under the licence named',
    'beside it above, and the usual text of that licence is among the texts below:',
    '',
    ...bare.sort(byName).map((p) => `  ${label(p)}  -  ${p.licence}`),
    '',
  );
}
parts.push(RULE, 'LICENCE TEXTS', RULE, '');
for (const { text, packages } of [...users.values()].sort((a, b) => b.packages.length - a.packages.length || a.text.localeCompare(b.text))) {
  const names = [...new Set(packages.map(label))].sort();
  parts.push(LINE, `For: ${names.join(', ')}`, LINE, '', text, '');
}

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, parts.join('\n'));
const size = Math.round(parts.join('\n').length / 1024);
console.log(`licences: ${ui.length} npm packages, ${shell.length} crates, ${users.size} texts, ${size} KB${problem ? ` (incomplete: ${problem})` : ''}`);
if (strict && (ui.length === 0 || shell.length === 0 || users.size < 5)) {
  console.error('licences: too little was found for this to be right');
  process.exit(1);
}
