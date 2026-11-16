<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, type ColumnEntry, type SchemaObjects } from '../lib/types';

  interface Props {
    workspaceId: number;
    /** The session's default schema, shown in bold. */
    current: string | null;
    /** Show the rows of a table or view. */
    onopen: (schema: string, table: string) => void;
    /** Make a schema the default for the active tab. */
    onuse: (schema: string) => void;
  }

  let { workspaceId, current, onopen, onuse }: Props = $props();

  type Loaded<T> = { state: 'loading' } | { state: 'error'; message: string } | { state: 'ready'; value: T };

  let schemas = $state<Loaded<string[]>>({ state: 'loading' });
  let objects = $state<Record<string, Loaded<SchemaObjects>>>({});
  let columns = $state<Record<string, Loaded<ColumnEntry[]>>>({});
  let open = $state<Record<string, boolean>>({});
  let filter = $state('');

  const needle = $derived(filter.trim().toLowerCase());

  async function loadSchemas() {
    schemas = { state: 'loading' };
    objects = {};
    columns = {};
    try {
      schemas = { state: 'ready', value: await backend.schemaList(workspaceId) };
    } catch (e) {
      schemas = { state: 'error', message: asUiError(e).message };
    }
  }

  async function toggleSchema(schema: string) {
    open[schema] = !open[schema];
    if (!open[schema] || objects[schema]) return;
    objects[schema] = { state: 'loading' };
    try {
      objects[schema] = { state: 'ready', value: await backend.schemaObjects(workspaceId, schema) };
    } catch (e) {
      objects[schema] = { state: 'error', message: asUiError(e).message };
    }
  }

  async function toggleTable(schema: string, table: string) {
    const key = `${schema}\u0000${table}`;
    open[key] = !open[key];
    if (!open[key] || columns[key]) return;
    columns[key] = { state: 'loading' };
    try {
      columns[key] = { state: 'ready', value: await backend.tableColumns(workspaceId, schema, table) };
    } catch (e) {
      columns[key] = { state: 'error', message: asUiError(e).message };
    }
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

  onMount(loadSchemas);
</script>

<div class="tree">
  <div class="toolbar">
    <input type="search" bind:value={filter} placeholder="Filter schemas and tables" aria-label="Filter schemas and tables" />
    <button class="quiet" title="Reload" aria-label="Reload schemas" onclick={loadSchemas}>&#8635;</button>
  </div>

  <div class="scroll">
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
          <li>
            <div class="row">
              <button
                class="node"
                class:current={schema === current}
                aria-expanded={!!open[schema]}
                title="Double-click to make this the default schema"
                onclick={() => toggleSchema(schema)}
                ondblclick={() => onuse(schema)}
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
                        {@const key = `${schema}\u0000${table.name}`}
                        {@const cols = columns[key]}
                        <li>
                          <div class="row">
                            <button
                              class="node"
                              aria-expanded={!!open[key]}
                              title="Double-click to show rows"
                              onclick={() => toggleTable(schema, table.name)}
                              ondblclick={() => onopen(schema, table.name)}
                            >
                              <span class="chevron" aria-hidden="true">{open[key] ? '▾' : '▸'}</span>
                              <span class="label">{table.name}</span>
                            </button>
                            <button
                              class="quiet action"
                              aria-label={`Show rows of ${table.name}`}
                              title="Show rows"
                              onclick={() => onopen(schema, table.name)}>&#9655;</button
                            >
                          </div>
                          {#if open[key]}
                            {#if !cols || cols.state === 'loading'}
                              <p class="muted note indent2">Loading&hellip;</p>
                            {:else if cols.state === 'error'}
                              <p class="error-text note indent2" role="alert">{cols.message}</p>
                            {:else}
                              <ul class="columns">
                                {#each cols.value as column (column.name)}
                                  <li title={`${column.columnType}${column.nullable ? '' : ' NOT NULL'}${column.extra ? ' ' + column.extra : ''}`}>
                                    <span class="label" class:key={column.key === 'PRI'}>{column.name}</span>
                                    <span class="type muted">{column.columnType}</span>
                                  </li>
                                {/each}
                              </ul>
                            {/if}
                          {/if}
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/each}
                {#each [{ title: 'Routines', items: loaded.value.routines.map((r) => r.name) }, { title: 'Triggers', items: loaded.value.triggers.map((t) => t.name) }, { title: 'Events', items: loaded.value.events }] as group (group.title)}
                  {@const items = group.items.filter((name) => nameVisible(schema, name))}
                  {#if items.length > 0}
                    <div class="group">{group.title} <span class="muted">{items.length}</span></div>
                    <ul class="leaves">
                      {#each items as name (name)}
                        <li><span class="label">{name}</span></li>
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

  .node:hover,
  .row:hover .node {
    background: var(--panel-2);
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

  .columns li,
  .leaves li {
    display: flex;
    gap: 8px;
    padding: 1px 8px 1px 24px;
    white-space: nowrap;
  }

  .leaves li {
    padding-left: 38px;
  }

  .columns .key {
    font-weight: 700;
  }

  .type {
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 12px;
  }

  .note {
    margin: 0;
    padding: 6px 10px;
    overflow-wrap: anywhere;
  }

  .indent {
    padding-left: 26px;
  }

  .indent2 {
    padding-left: 40px;
  }
</style>
