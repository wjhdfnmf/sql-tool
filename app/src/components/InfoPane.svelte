<script lang="ts">
  import { backend } from '../lib/backend';
  import type { WorkspaceState } from '../lib/state.svelte';
  import { asUiError, serverLabel, type ColumnEntry, type SchemaDefaults, type TableKeys } from '../lib/types';

  interface Props {
    workspace: WorkspaceState;
    /** A column was double-clicked: write its name into the editor. */
    oninsert: (name: string) => void;
  }

  let { workspace, oninsert }: Props = $props();

  let tab = $state<'object' | 'session'>('object');
  let columns = $state<ColumnEntry[]>([]);
  let defaults = $state<SchemaDefaults | null>(null);
  let keys = $state<TableKeys | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  const info = $derived(workspace.info);
  const target = $derived(workspace.selected);

  const isTable = $derived(!!target && (target.kind === 'table' || target.kind === 'view'));

  $effect(() => {
    const current = target;
    columns = [];
    defaults = null;
    keys = null;
    error = null;
    if (!current) return;
    let stale = false;
    loading = true;
    (async () => {
      try {
        if (current.kind === 'schema') {
          const value = await backend.schemaDefaults(info.id, current.schema);
          if (!stale) defaults = value;
        } else if ((current.kind === 'table' || current.kind === 'view') && current.name) {
          const [c, k] = await Promise.all([
            backend.tableColumns(info.id, current.schema, current.name),
            current.kind === 'table' ? backend.tableKeys(info.id, current.schema, current.name) : null,
          ]);
          if (!stale) {
            columns = c;
            keys = k;
          }
        }
      } catch (e) {
        if (!stale) error = asUiError(e).message;
      } finally {
        if (!stale) loading = false;
      }
    })();
    // A newer selection makes this answer irrelevant.
    return () => {
      stale = true;
    };
  });

  const security = $derived(
    info.security === 'verified'
      ? 'Server verified'
      : info.security === 'pinned'
        ? 'Pinned certificate'
        : info.security === 'unverified'
          ? 'Server not verified'
          : 'Not encrypted',
  );
</script>

<section class="info">
  <div class="tabs" role="tablist" aria-label="Information">
    <button role="tab" aria-selected={tab === 'object'} class:active={tab === 'object'} onclick={() => (tab = 'object')}>
      Object info
    </button>
    <button role="tab" aria-selected={tab === 'session'} class:active={tab === 'session'} onclick={() => (tab = 'session')}>
      Session
    </button>
  </div>

  <div class="body" hidden={tab !== 'object'}>
    {#if !target}
      <p class="muted">Click a table above to see its columns and indexes.</p>
    {:else}
      {#if error}<p class="error-text" role="alert">{error}</p>{/if}
      {#if loading}<p class="muted">Loading&hellip;</p>{/if}
      {#if !isTable && !defaults && !loading && !error}
        <p class="muted">Click a table above to see its columns and indexes.</p>
      {/if}

      {#if defaults}
        <dl>
          <dt>Character set</dt>
          <dd>{defaults.charset}</dd>
          <dt>Collation</dt>
          <dd>{defaults.collation}</dd>
        </dl>
      {/if}

      {#if columns.length > 0}
        <table>
          <thead>
            <tr><th>Column</th><th>Type</th><th>Null</th><th>Key</th></tr>
          </thead>
          <tbody>
            {#each columns as column (column.name)}
              <tr
                title={[column.default !== null ? `default ${column.default}` : '', column.extra, 'Double-click to insert the name into the editor']
                  .filter(Boolean)
                  .join('. ')}
                ondblclick={() => oninsert(column.name)}
              >
                <td class:key={column.key === 'PRI'}>{column.name}</td>
                <td class="muted">{column.columnType}</td>
                <td class="muted">{column.nullable ? 'yes' : 'no'}</td>
                <td class="muted">{column.key}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}

      {#if keys && (keys.indexes.length > 0 || keys.primaryKey.length > 0)}
        <h4>Indexes</h4>
        <dl class="stacked">
          {#if keys.primaryKey.length > 0}
            <dt>PRIMARY</dt>
            <dd>{keys.primaryKey.join(', ')}</dd>
          {/if}
          {#each keys.indexes as index (index.name)}
            <dt>{index.name}{index.unique ? ' (unique)' : ''}</dt>
            <dd>{index.columns.join(', ')}</dd>
          {/each}
        </dl>
      {/if}

      {#if keys && keys.foreignKeys.length > 0}
        <h4>Foreign keys</h4>
        <dl class="stacked">
          {#each keys.foreignKeys as key (key.name)}
            <dt>{key.name}</dt>
            <dd>
              {key.columns.join(', ')} &rarr; {key.referencedTable} ({key.referencedColumns.join(', ')})
              <span class="muted">on delete {key.onDelete.toLowerCase()}</span>
            </dd>
          {/each}
        </dl>
      {/if}
    {/if}
  </div>

  <div class="body" hidden={tab !== 'session'}>
    <dl>
      <dt>Connection</dt>
      <dd>{info.name}</dd>
      <dt>Host</dt>
      <dd>{info.host}</dd>
      <dt>Logged in as</dt>
      <dd>{info.info.currentUser}</dd>
      <dt>Server</dt>
      <dd>{serverLabel(info.info.version)}</dd>
      <dt>Version</dt>
      <dd>{info.info.version.raw}</dd>
      {#if info.info.versionComment}
        <dt>Edition</dt>
        <dd>{info.info.versionComment}</dd>
      {/if}
      <dt>Connection id</dt>
      <dd>{info.info.connectionId}</dd>
      <dt>Encryption</dt>
      <dd>{info.info.tlsVersion ? info.info.tlsVersion.replace('TLSv', 'TLS ') : 'None'}</dd>
      {#if info.info.tlsCipher}
        <dt>Cipher</dt>
        <dd>{info.info.tlsCipher}</dd>
      {/if}
      <dt>Verification</dt>
      <dd>{security}</dd>
      <dt>Default schema</dt>
      <dd>{workspace.database ?? 'none selected'}</dd>
    </dl>
  </div>
</section>

<style>
  .info {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .tabs {
    flex: none;
    display: flex;
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    flex: 1;
    border: none;
    border-radius: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    padding: 5px;
    color: var(--muted);
  }

  .tabs button.active {
    border-bottom-color: var(--accent);
    color: var(--text);
    font-weight: 600;
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 8px 10px 12px;
    user-select: text;
  }

  p {
    margin: 2px 0 6px;
  }

  dl {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 2px 10px;
    margin: 8px 0;
  }

  dt {
    color: var(--muted);
    white-space: nowrap;
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  h4 {
    margin: 12px 0 2px;
    font-size: 12px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  dl.stacked {
    display: block;
    margin: 4px 0;
    font-size: 12px;
  }

  dl.stacked dt {
    color: var(--text);
    font-weight: 600;
    white-space: normal;
    overflow-wrap: anywhere;
  }

  dl.stacked dd {
    margin: 0 0 4px 10px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  th {
    text-align: left;
    font-weight: 600;
    color: var(--muted);
    border-bottom: 1px solid var(--border);
    padding: 3px 6px 3px 0;
  }

  td {
    padding: 2px 6px 2px 0;
    overflow-wrap: anywhere;
    vertical-align: top;
  }

  td.key {
    font-weight: 700;
  }
</style>
