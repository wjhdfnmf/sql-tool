<script lang="ts">
  import { adminQueries } from '../lib/admin';

  interface Props {
    /** Open a new query tab with this statement and run it. */
    onrun: (title: string, sql: string) => void;
  }

  let { onrun }: Props = $props();
</script>

<div class="admin">
  <p class="muted">Each item opens a query tab and runs the statement shown.</p>
  <ul>
    {#each adminQueries as item (item.title)}
      <li>
        <button onclick={() => onrun(item.title, item.sql)}>
          <span class="title">{item.title}</span>
          <code>{item.sql}</code>
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .admin {
    height: 100%;
    overflow: auto;
  }

  p {
    margin: 0;
    padding: 8px 10px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  button {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    border-radius: 0;
    background: transparent;
    text-align: left;
    white-space: normal;
  }

  button:hover {
    background: var(--panel-2);
  }

  .title {
    font-weight: 600;
  }

  code {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
</style>
