// The licences of the software the app is built with.
//
// Collected into one text file when the interface is built (see
// `tools/licences.mjs`), and loaded only when asked for: it is long.

export async function loadLicences(): Promise<string> {
  return (await import('../generated/licences.txt?raw')).default;
}
