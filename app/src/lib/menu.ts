// Shapes for the menu bar and right-click menus.

export type MenuItem =
  | { separator: true }
  | {
      label: string;
      /** Shown on the right, for example "Ctrl+Enter". Display only. */
      shortcut?: string;
      disabled?: boolean;
      /** Draws a check mark; for on/off and pick-one items. */
      checked?: boolean;
      action: () => void;
    };

export interface MenuDef {
  label: string;
  items: MenuItem[];
}

export const separator: MenuItem = { separator: true };
