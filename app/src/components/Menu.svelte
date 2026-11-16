<script lang="ts">
  import { onMount } from 'svelte';
  import type { MenuItem } from '../lib/menu';

  interface Props {
    items: MenuItem[];
    /** Where the menu's top-left corner goes, in viewport pixels. */
    x: number;
    y: number;
    onclose: () => void;
    /** Left/right arrow pressed; a menu bar uses this to move to the next menu. */
    onsideways?: (direction: -1 | 1) => void;
    label: string;
  }

  let { items, x, y, onclose, onsideways, label }: Props = $props();

  let menu: HTMLDivElement;
  let left = $state(0);
  let top = $state(0);

  function buttons(): HTMLButtonElement[] {
    return Array.from(menu.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
  }

  function move(delta: number) {
    const list = buttons();
    if (list.length === 0) return;
    const current = list.indexOf(document.activeElement as HTMLButtonElement);
    list[(current + delta + list.length) % list.length].focus();
  }

  function onkeydown(e: KeyboardEvent) {
    switch (e.key) {
      case 'ArrowDown': move(1); break;
      case 'ArrowUp': move(-1); break;
      case 'Home': buttons()[0]?.focus(); break;
      case 'End': buttons().at(-1)?.focus(); break;
      case 'ArrowLeft': onsideways?.(-1); break;
      case 'ArrowRight': onsideways?.(1); break;
      case 'Escape':
      case 'Tab':
        onclose();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  onMount(() => {
    // Keep the menu inside the window.
    const rect = menu.getBoundingClientRect();
    left = Math.max(4, Math.min(x, window.innerWidth - rect.width - 4));
    top = Math.max(4, Math.min(y, window.innerHeight - rect.height - 4));
    buttons()[0]?.focus();

    const outside = (e: Event) => {
      if (!menu.contains(e.target as Node)) onclose();
    };
    // Capture phase, so the click that closes the menu is seen before
    // whatever it lands on reacts to it.
    window.addEventListener('pointerdown', outside, true);
    window.addEventListener('blur', onclose);
    window.addEventListener('resize', onclose);
    return () => {
      window.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('blur', onclose);
      window.removeEventListener('resize', onclose);
    };
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="menu"
  role="menu"
  aria-label={label}
  tabindex="-1"
  bind:this={menu}
  style:left={`${left}px`}
  style:top={`${top}px`}
  {onkeydown}
  oncontextmenu={(e) => e.preventDefault()}
>
  {#each items as item}
    {#if 'separator' in item}
      <div class="separator" role="separator"></div>
    {:else}
      <button
        role="menuitem"
        disabled={item.disabled}
        onclick={() => {
          onclose();
          item.action();
        }}
      >
        <span class="check" aria-hidden="true">{item.checked ? '✓' : ''}</span>
        <span class="label">{item.label}</span>
        {#if item.shortcut}<span class="shortcut">{item.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 100;
    min-width: 200px;
    max-width: 360px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.22);
  }

  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 10px 5px 6px;
    border: none;
    border-radius: 4px;
    background: transparent;
    text-align: left;
  }

  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--accent);
    color: var(--accent-text);
    outline: none;
  }

  button:hover:not(:disabled) .shortcut,
  button:focus-visible .shortcut {
    color: inherit;
  }

  .check {
    flex: none;
    width: 14px;
    text-align: center;
  }

  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .shortcut {
    flex: none;
    padding-left: 20px;
    color: var(--muted);
    font-size: 12px;
  }

  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
