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
    /** The + beside a group: make a new object of that kind in the schema. */
    oncreate: (kind: ObjectKind, schema: string) => void;
  }

  let { workspaceId, current, version, selected, onselect, onopen, onuse, oncontext, onloaded, oncreate }: Props = $props();

  type Loaded<T> = { state: 'loading' } | { state: 'error'; message: string } | { state: 'ready'; value: T };

  let schemas = $state<Loaded<string[]>>({ state: 'loading' });
  let objects = $state<Record<string, Loaded<SchemaObjects>>>({});
  let open = $state<Record<string, boolean>>({});
  let filter = $state('');

  const needle = $derived(filter.trim().toLowerCase());
  /** `schema.table` narrows by both; anything without a dot matches either. */
  const dotted = $derived.by(() => {
    const dot = needle.indexOf('.');
    return dot < 0 ? null : { schema: needle.slice(0, dot), name: needle.slice(dot + 1) };
  });

  // Filtering as schema.table has to look inside the schemas that match,
  // so those are loaded and shown open. Only a handful: a filter that fits
  // a hundred schemas should not send a hundred requests.
  $effect(() => {
    const wanted = dotted;
    const list = schemas.state === 'ready' ? schemas.value : [];
    if (!wanted) return;
    untrack(() => {
      for (const schema of list.filter((s) => s.toLowerCase().includes(wanted.schema)).slice(0, 8)) {
        if (!objects[schema]) void loadObjects(schema);
      }
    });
  });

  function isOpen(schema: string): boolean {
    return !!open[schema] || (dotted !== null && !!objects[schema]);
  }

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

  /** With a filter, a schema is shown if its name or one of its loaded tables matches. */
  function schemaVisible(schema: string): boolean {
    if (dotted) return schema.toLowerCase().includes(dotted.schema);
    if (!needle || schema.toLowerCase().includes(needle)) return true;
    const loaded = objects[schema];
    return loaded?.state === 'ready' && loaded.value.tables.some((t) => t.name.toLowerCase().includes(needle));
  }

  function nameVisible(schema: string, name: string): boolean {
    if (dotted) return name.toLowerCase().includes(dotted.name);
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

  interface Group {
    title: string;
    /** What the + beside the title makes. */
    kind: ObjectKind;
    /** A new one of these: "table", "stored procedure". */
    noun: string;
    items: { name: string; kind: ObjectKind }[];
    /** Shown even when it holds nothing, so there is always a place to add one. */
    always: boolean;
  }

  /** Everything in a schema, group by group, after the filter. */
  function groups(schema: string, loaded: SchemaObjects): Group[] {
    const seen = (name: string) => nameVisible(schema, name);
    const named = (kind: ObjectKind, names: string[]) => names.filter(seen).map((name) => ({ name, kind }));
    const all: Group[] = [
      { title: 'Tables', kind: 'table', noun: 'table', always: true, items: named('table', loaded.tables.filter((t) => !t.isView).map((t) => t.name)) },
      { title: 'Views', kind: 'view', noun: 'view', always: true, items: named('view', loaded.tables.filter((t) => t.isView).map((t) => t.name)) },
      {
        title: 'Stored procedures', kind: 'procedure', noun: 'stored procedure', always: true,
        items: named('procedure', loaded.routines.filter((r) => r.kind === 'PROCEDURE').map((r) => r.name)),
      },
      {
        title: 'Functions', kind: 'function', noun: 'function', always: true,
        items: named('function', loaded.routines.filter((r) => r.kind !== 'PROCEDURE').map((r) => r.name)),
      },
      { title: 'Triggers', kind: 'trigger', noun: 'trigger', always: false, items: named('trigger', loaded.triggers.map((t) => t.name)) },
      { title: 'Events', kind: 'event', noun: 'event', always: false, items: named('event', loaded.events) },
    ];
    // While filtering, groups with nothing that matches only get in the way.
    return all.filter((group) => group.items.length > 0 || (group.always && !needle));
  }
</script>

<div class="tree">
  <div class="toolbar">
    <input
      type="search"
      bind:value={filter}
      placeholder="Filter: table, or schema.table"
      aria-label="Filter schemas and tables"
      title="Type part of a name. With a dot, the part before it is the schema and the part after it the table: sales.ord"
    />
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
                aria-expanded={isOpen(schema)}
                title="Double-click to make this the default schema. Right-click for more."
                onclick={() => toggleSchema(schema)}
                ondblclick={() => onuse(schema)}
                oncontextmenu={(e) => context(e, schemaTarget)}
                onkeydown={(e) => menuKey(e, schemaTarget)}
              >
                <span class="chevron" aria-hidden="true">{isOpen(schema) ? '▾' : '▸'}</span>
                <span class="label">{schema}</span>
              </button>
            </div>
            {#if isOpen(schema)}
              {#if !loaded || loaded.state === 'loading'}
                <p class="muted note indent">Loading&hellip;</p>
              {:else if loaded.state === 'error'}
                <p class="error-text note indent" role="alert">{loaded.message}</p>
              {:else}
                {#each groups(schema, loaded.value) as group (group.title)}
                  <div class="group">
                    <span>{group.title} <span class="muted">{group.items.length}</span></span>
                    <button
                      class="quiet plus"
                      aria-label={`New ${group.noun} in ${schema}`}
                      title={`New ${group.noun} in ${schema}`}
                      onclick={() => oncreate(group.kind, schema)}>+</button
                    >
                  </div>
                  {#if group.items.length > 0}
                    <ul>
                      {#each group.items as item (item.name)}
                        {@const target = { kind: item.kind, schema, name: item.name }}
                        {@const rows = item.kind === 'table' || item.kind === 'view'}
                        <li>
                          <div class="row" class:selected={isSelected(item.kind, schema, item.name)}>
                            <button
                              class="node leaf"
                              title={rows
                                ? 'Click to see its columns and indexes below. Double-click to show rows. Right-click for more.'
                                : 'Right-click for more'}
                              onclick={() => onselect(target)}
                              ondblclick={() => {
                                if (rows) onopen(schema, item.name);
                              }}
                              oncontextmenu={(e) => context(e, target)}
                              onkeydown={(e) => menuKey(e, target)}
                            >
                              <span class="label">{item.name}</span>
                            </button>
                            {#if rows}
                              <button
                                class="quiet action"
                                aria-label={`Show rows of ${item.name}`}
                                title="Show rows"
                                onclick={() => onopen(schema, item.name)}>&#9655;</button
                              >
                            {/if}
                          </div>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/each}
                {#if needle && groups(schema, loaded.value).length === 0}
                  <p class="muted note indent">Nothing here matches.</p>
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
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 6px 0 22px;
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

  .plus {
    padding: 0 7px;
    font-size: 15px;
    line-height: 18px;
    color: var(--muted);
    /* The title beside it is upper case and small; the button is neither. */
    text-transform: none;
    letter-spacing: 0;
  }
</style>
