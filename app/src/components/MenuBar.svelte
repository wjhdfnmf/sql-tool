<script lang="ts">
  import type { MenuDef } from '../lib/menu';
  import Menu from './Menu.svelte';

  interface Props {
    menus: MenuDef[];
  }

  let { menus }: Props = $props();

  let open = $state<number | null>(null);
  let position = $state({ x: 0, y: 0 });
  let buttons: HTMLButtonElement[] = $state([]);

  function show(index: number) {
    const rect = buttons[index].getBoundingClientRect();
    position = { x: rect.left, y: rect.bottom + 2 };
    open = index;
  }

  // Pressing the open menu's own button first closes the menu (the press
  // lands outside it) and then clicks the button; remember the close so
  // that click does not immediately reopen it.
  let lastClosed = { index: -1, at: 0 };

  function close(refocus = false) {
    const index = open;
    open = null;
    if (index !== null) lastClosed = { index, at: performance.now() };
    if (refocus && index !== null) buttons[index]?.focus();
  }

  function toggle(index: number) {
    if (open === index) close();
    else if (lastClosed.index !== index || performance.now() - lastClosed.at > 300) show(index);
  }

  function sideways(direction: -1 | 1) {
    if (open === null) return;
    show((open + direction + menus.length) % menus.length);
  }

  function onkeydown(e: KeyboardEvent, index: number) {
    if (e.key === 'ArrowRight') buttons[(index + 1) % menus.length].focus();
    else if (e.key === 'ArrowLeft') buttons[(index - 1 + menus.length) % menus.length].focus();
    else if (e.key === 'ArrowDown') show(index);
    else return;
    e.preventDefault();
  }
</script>

<div class="menubar" role="menubar" aria-label="Application menu">
  {#each menus as menu, index (menu.label)}
    <button
      role="menuitem"
      aria-haspopup="menu"
      aria-expanded={open === index}
      class:open={open === index}
      bind:this={buttons[index]}
      onclick={() => toggle(index)}
      onpointerenter={() => {
        if (open !== null && open !== index) show(index);
      }}
      onkeydown={(e) => onkeydown(e, index)}
    >
      {menu.label}
    </button>
  {/each}
</div>

{#if open !== null}
  {#key open}
    <Menu
      items={menus[open].items}
      label={menus[open].label}
      x={position.x}
      y={position.y}
      onclose={() => close(true)}
      onsideways={sideways}
    />
  {/key}
{/if}

<style>
  .menubar {
    flex: none;
    display: flex;
    gap: 2px;
    padding: 2px 6px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }

  button {
    padding: 3px 9px;
    border: none;
    border-radius: 4px;
    background: transparent;
  }

  button:hover,
  button.open {
    background: var(--panel-2);
  }
</style>
