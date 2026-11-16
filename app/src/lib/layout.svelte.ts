// Sizes of the panes the user can drag, remembered on this computer.

const KEY = 'sqltool.layout';

interface Layout {
  /** Width of the navigator (schemas and object info), in pixels. */
  navWidth: number;
  /** Height of the object info pane under the schema tree, in pixels. */
  infoHeight: number;
}

const defaults: Layout = { navWidth: 290, infoHeight: 240 };

function load(): Layout {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    const read = (name: keyof Layout, low: number, high: number) => {
      const value = (saved as Record<string, unknown>)?.[name];
      return typeof value === 'number' && Number.isFinite(value) ? Math.min(Math.max(value, low), high) : defaults[name];
    };
    return { navWidth: read('navWidth', 160, 900), infoHeight: read('infoHeight', 60, 1200) };
  } catch {
    return { ...defaults };
  }
}

export const layout = $state<Layout>(load());

export function saveLayout(): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(layout));
  } catch {
    // Remembering sizes is a convenience.
  }
}
