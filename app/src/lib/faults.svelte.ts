// Errors that nothing handled.
//
// Every call that can fail is meant to say so where it was made. This is for
// what slips through: without it, a mistake in the interface shows as a
// button that does nothing, and an installed app has no console to look in.
// The user is told that something went wrong, and a line goes to crash.log.

import { backend } from './backend';

export const faults = $state<{
  /** What to tell the user, until they dismiss it. */
  message: string | null;
  /** Everything caught in this run, for the app's own tests to check. */
  seen: string[];
}>({ message: null, seen: [] });

/** Browser noise that is not a fault. */
const NOISE = /ResizeObserver loop/;
/** A call made for something that has since been closed: there is nothing to tell. */
const GONE = new Set(['not-connected', 'no-result']);

/** Where in the app's code an error came from: the first place in its stack, without the address of the page. */
function origin(stack: string | undefined): string {
  const frame = (stack ?? '').split('\n').find((line) => /:\d+:\d+/.test(line)) ?? '';
  return frame
    .trim()
    .replace(/^at\s+/, '')
    .replace(/[a-z-]+:\/\/[^/\s)]+/gi, '')
    .slice(0, 160);
}

function note(reason: unknown): void {
  let shown: string;
  let logged: string;
  if (reason instanceof Error) {
    if (NOISE.test(reason.message)) return;
    shown = `${reason.name}: ${reason.message}`.slice(0, 300);
    logged = `${reason.name}: ${reason.message.slice(0, 200)} at ${origin(reason.stack)}`;
  } else if (typeof reason === 'object' && reason !== null && 'kind' in reason && 'message' in reason) {
    // What a backend call rejects with. Its message may quote a statement,
    // so the user sees it and the log gets only the kind.
    const { kind, message } = reason as { kind: unknown; message: unknown };
    if (GONE.has(String(kind))) return;
    shown = String(message).slice(0, 300);
    logged = `a backend error of kind ${String(kind)} was not handled`;
  } else {
    const text = String(reason);
    if (NOISE.test(text)) return;
    shown = text.slice(0, 300);
    logged = `an error that is not an Error was not handled (${typeof reason})`;
  }
  faults.seen.push(logged);
  faults.message = shown;
  void backend.uiFault(logged).catch(() => {});
}

/** Starts watching. Called once, before the interface is drawn. */
export function watchForFaults(): void {
  window.addEventListener('error', (e) => note(e.error ?? e.message));
  window.addEventListener('unhandledrejection', (e) => note(e.reason));
}
