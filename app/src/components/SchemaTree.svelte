<script lang="ts">
  import { untrack } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, type ObjectKind, type SchemaObjects, type TreeTarget } from '../lib/types';

  interface Props {
    workspaceId: number;
    /** The session's default schema, shown in bold. */
    current: string | null;
    /** Changes when the tree should reload from the server. */
    version: number;
    selected: TreeTarget | null;
    onselect: (target: TreeTarget) => void;
    /** Show the rows of a table or view. */
    onopen: (schema: string, table: string) => void;
    /** Make a schema the default for the active tab. */
    onuse: (schema: string) => void;
    /** A right-click (or the menu key). `target` is null on empty space. */
    oncontext: (target: TreeTarget | null, x: number, y: number) => void;
    /** A schema was expanded; its names are worth loading for completion. */
    onloaded: (schema: string) => void;
  }

  let { workspaceId, current, version, selected, onselect, onopen, onuse, oncontext, onloaded }: Props = $props();

  type Loaded<T> = { state: 'loading' } | { state: 'error'; message: string } | { state: 'ready'; value: T };

  let schemas = $state<Loaded<string[]>>({ state: 'loading' });
  let objects = $state<Record<string, Loaded<SchemaObjects>>>({});
  let open = $state<Record<string, boolean>>({});
  let filter = $state('');

  const needle = $derived(filter.trim().toLowerCase());

  async function loadObjects(schema: string) {
    objects[schema] = { state: 'loading' };
    try {
      objects[schema] = { state: 'ready', value: await backend.schemaObjects(workspaceId, schema) };
      onloaded(schema);
    } catch (e) {
      objects[schema] = { state: 'error', message: asUiError(e).message };
    }
  }

  async function loadSchemas() {
    // Keep what is expanded; reload what was already loaded.
    const expanded = Object.keys(objects).filter((schema) => open[schema]);
    schemas = { state: 'loading' };
    objects = {};
    try {
      const list = await backend.schemaList(workspaceId);
      schemas = { state: 'ready', value: list };
      for (const schema of expanded) {
        if (list.includes(schema)) void loadObjects(schema);
      }
    } catch (e) {
      schemas = { state: 'error', message: asUiError(e).message };
    }
  }

  $effect(() => {
    void version;
    // Reload when `version` changes only. Loading reads and writes the tree's
    // own state, which must not make this effect run again.
    untrack(() => void loadSchemas());
  });

  function toggleSchema(schema: string) {
    onselect({ kind: 'schema', schema, name: null });
    open[schema] = !open[schema];
    if (open[schema] && !objects[schema]) void loadObjects(schema);
  }

  /** Clicking a table selects it; the information pane shows its columns and indexes. */
  function selectTable(schema: string, table: string, isView: boolean) {
    onselect({ kind: isView ? 'view' : 'table', schema, name: table });
  }

  /** With a filter, a schema is shown if its name or one of its loaded tables matches. */
  function schemaVisible(schema: string): boolean {
    if (!needle || schema.toLowerCase().includes(needle)) return true;
    const loaded = objects[schema];
    return loaded?.state === 'ready' && loaded.value.tables.some((t) => t.name.toLowerCase().includes(needle));
  }

  function nameVisible(schema: string, name: string): boolean {
    return !needle || schema.toLowerCase().includes(needle) || name.toLowerCase().includes(needle);
  }

  function isSelected(kind: TreeTarget['kind'], schema: string, name: string | null): boolean {
    return selected?.kind === kind && selected.schema === schema && selected.name === name;
  }

  function context(e: MouseEvent, target: TreeTarget | null) {
    e.preventDefault();
    e.stopPropagation();
    if (target) onselect(target);
    oncontext(target, e.clientX, e.clientY);
  }

  /** The menu key, or Shift+F10, opens the same menu a right-click does. */
  function menuKey(e: KeyboardEvent, target: TreeTarget) {
    if (e.key !== 'ContextMenu' && !(e.key === 'F10' && e.shiftKey)) return;
    e.preventDefault();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onselect(target);
    oncontext(target, rect.left + 24, rect.bottom);
  }

  function leaves(loaded: SchemaObjects): { title: string; items: { name: string; kind: ObjectKind }[] }[] {
    return [
      {
        title: 'Stored procedures',
        items: loaded.routines.filter((r) => r.kind === 'PROCEDURE').map((r) => ({ name: r.name, kind: 'procedure' as const })),
      },
      {
        title: 'Functions',
        items: loaded.routines.filter((r) => r.kind !== 'PROCEDURE').map((r) => ({ name: r.name, kind: 'function' as const })),
      },
      { title: 'Triggers', items: loaded.triggers.map((t) => ({ name: t.name, kind: 'trigger' as const })) },
      { title: 'Events', items: loaded.events.map((name) => ({ name, kind: 'event' as const })) },
    ];
  }
</script>

<div class="tree">
  <div class="toolbar">
    <input type="search" bind:value={filter} placeholder="Filter objects" aria-label="Filter schemas and tables" />
    <button class="quiet" title="Reload" aria-label="Reload schemas" onclick={loadSchemas}>&#8635;</button>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scroll" oncontextmenu={(e) => context(e, null)}>
    {#if schemas.state === 'loading'}
      <p class="muted note">Loading schemas&hellip;</p>
    {:else if schemas.state === 'error'}
      <p class="error-text note" role="alert">{schemas.message}</p>
    {:else if schemas.value.length === 0}
      <p class="muted note">No schemas are visible to this account.</p>
    {:else}
      <ul>
        {#each schemas.value.filter(schemaVisible) as schema (schema)}
          {@const loaded = objects[schema]}
          {@const schemaTarget = { kind: 'schema' as const, schema, name: null }}
          <li>
            <div class="row" class:selected={isSelected('schema', schema, null)}>
              <button
                class="node"
                class:current={schema === current}
                aria-expanded={!!open[schema]}
                title="Double-click to make this the default schema. Right-click for more."
                onclick={() => toggleSchema(schema)}
                ondblclick={() => onuse(schema)}
                oncontextmenu={(e) => context(e, schemaTarget)}
                onkeydown={(e) => menuKey(e, schemaTarget)}
              >
                <span class="chevron" aria-hidden="true">{open[schema] ? '▾' : '▸'}</span>
                <span class="label">{schema}</span>
              </button>
            </div>
            {#if open[schema]}
              {#if !loaded || loaded.state === 'loading'}
                <p class="muted note indent">Loading&hellip;</p>
              {:else if loaded.state === 'error'}
                <p class="error-text note indent" role="alert">{loaded.message}</p>
              {:else}
                {@const tables = loaded.value.tables.filter((t) => !t.isView && nameVisible(schema, t.name))}
                {@const views = loaded.value.tables.filter((t) => t.isView && nameVisible(schema, t.name))}
                {#each [{ title: 'Tables', items: tables }, { title: 'Views', items: views }] as group (group.title)}
                  {#if group.items.length > 0}
                    <div class="group">{group.title} <span class="muted">{group.items.length}</span></div>
                    <ul>
                      {#each group.items as table (table.name)}
                        {@const kind = table.isView ? ('view' as const) : ('table' as const)}
                        {@const target = { kind, schema, name: table.name }}
                        <li>
                          <div class="row" class:selected={isSelected(kind, schema, table.name)}>
                            <button
                              class="node"
                              title="Click to see its columns and indexes below. Double-click to show rows. Right-click for more."
                              onclick={() => selectTable(schema, table.name, table.isView)}
                              ondblclick={() => onopen(schema, table.name)}
                              oncontextmenu={(e) => context(e, target)}
                              onkeydown={(e) => menuKey(e, target)}
                            >
                              <span class="chevron" aria-hidden="true"></span>
                              <span class="label">{table.name}</span>
                            </button>
                            <button
                              class="quiet action"
                              aria-label={`Show rows of ${table.name}`}
                              title="Show rows"
                              onclick={() => onopen(schema, table.name)}>&#9655;</button
                            >
                          </div>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/each}
                {#each leaves(loaded.value) as group (group.title)}
                  {@const items = group.items.filter((item) => nameVisible(schema, item.name))}
                  {#if items.length > 0}
                    <div class="group">{group.title} <span class="muted">{items.length}</span></div>
                    <ul class="leaves">
                      {#each items as item (item.name)}
                        {@const target = { kind: item.kind, schema, name: item.name }}
                        <li>
                          <div class="row" class:selected={isSelected(item.kind, schema, item.name)}>
                            <button
                              class="node leaf"
                              title="Right-click for more"
                              onclick={() => onselect(target)}
                              oncontextmenu={(e) => context(e, target)}
                              onkeydown={(e) => menuKey(e, target)}
                            >
                              <span class="label">{item.name}</span>
                            </button>
                          </div>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/each}
                {#if loaded.value.tables.length + loaded.value.routines.length + loaded.value.triggers.length + loaded.value.events.length === 0}
                  <p class="muted note indent">Empty schema.</p>
                {/if}
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .tree {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .toolbar {
    flex: none;
    display: flex;
    gap: 4px;
    padding: 8px;
  }

  .toolbar input {
    flex: 1;
  }

  .toolbar button {
    padding: 2px 8px;
    font-size: 15px;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-bottom: 12px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  ul ul {
    padding-left: 14px;
  }

  .row {
    display: flex;
    align-items: center;
  }

  .node {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border: none;
    border-radius: 0;
    background: transparent;
    text-align: left;
  }

  .node.leaf {
    padding-left: 24px;
  }

  .row:hover {
    background: var(--panel-2);
  }

  .row.selected {
    background: var(--selection);
  }

  .node.current .label {
    font-weight: 700;
  }

  .chevron {
    flex: none;
    width: 12px;
    color: var(--muted);
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .action {
    flex: none;
    padding: 0 8px;
    font-size: 11px;
    color: var(--muted);
    opacity: 0;
  }

  .row:hover .action,
  .action:focus-visible {
    opacity: 1;
  }

  .group {
    padding: 6px 8px 2px 22px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  .note {
    margin: 0;
    padding: 6px 10px;
    overflow-wrap: anywhere;
  }

  .indent {
    padding-left: 26px;
  }
</style>
