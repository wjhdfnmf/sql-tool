<script lang="ts">
  // One filter of a graph: a range for numbers and times, a list to tick for categories.

  import { backend } from '../lib/backend';
  import { NULL_LABEL, ROLE_LABELS, formatNumber, formatTime, parseTime, type ChartFilter, type ChartRole, type ColumnSummary } from '../lib/chart';
  import type { GraphState } from '../lib/graph.svelte';
  import { asUiError, formatCount } from '../lib/types';
  import RangeSlider from './RangeSlider.svelte';

  interface Props {
    graph: GraphState;
    filter: ChartFilter;
    onremove: () => void;
  }

  let { graph, filter, onremove }: Props = $props();

  let summary = $state.raw<ColumnSummary | null>(null);
  let error = $state('');
  let search = $state('');

  const name = $derived(graph.name(filter.column));
  const roles = $derived(graph.rolesOf(filter.column));

  $effect(() => {
    const [resultId, column, role] = [graph.resultId, filter.column, filter.role];
    let stale = false;
    summary = null;
    error = '';
    backend
      .chartColumn(resultId, column, role)
      .then((found) => {
        if (!stale) summary = found;
      })
      .catch((e) => {
        if (!stale) error = asUiError(e).message;
      });
    return () => {
      stale = true;
    };
  });

  function setRole(role: ChartRole) {
    filter.role = role;
    filter.min = null;
    filter.max = null;
    filter.values = [];
    search = '';
  }

  // ---- a range ----

  const ranged = $derived(summary !== null && summary.min !== null && summary.max !== null && filter.role !== 'discrete');
  const low = $derived(Math.max(summary?.min ?? 0, filter.min ?? -Infinity));
  const high = $derived(Math.min(summary?.max ?? 0, filter.max ?? Infinity));
  const show = (value: number) => (filter.role === 'time' ? formatTime(value, summary?.timeOnly) : formatNumber(value));

  /** A bound at the end of the data is no bound: the filter then keeps rows that have no value too. */
  function setRange(from: number, to: number) {
    if (!summary || summary.min === null || summary.max === null) return;
    filter.min = from <= summary.min ? null : from;
    filter.max = to >= summary.max ? null : to;
  }

  function typed(which: 'min' | 'max', text: string) {
    const value = text.trim() === '' ? null : filter.role === 'time' ? parseTime(text) : Number(text.replaceAll(',', ''));
    if (value === null || !Number.isFinite(value)) {
      filter[which] = null;
      return;
    }
    if (which === 'min') setRange(Math.min(value, high), high);
    else setRange(low, Math.max(value, low));
  }

  // ---- a list ----

  const listed = $derived.by(() => {
    const all = summary?.categories ?? [];
    const needle = search.trim().toLowerCase();
    return needle === '' ? all : all.filter((c) => (c.label ?? NULL_LABEL).toLowerCase().includes(needle));
  });

  function toggle(label: string | null, on: boolean) {
    filter.values = on ? [...filter.values, label] : filter.values.filter((v) => v !== label);
  }
</script>

<section class="filter" aria-label={`Filter on ${name}`}>
  <header>
    <span class="name" title={name}>{name}</span>
    {#if roles.length > 1}
      <select aria-label={`How ${name} is read`} value={filter.role} onchange={(e) => setRole(e.currentTarget.value as ChartRole)}>
        {#each roles as role}<option value={role}>{ROLE_LABELS[role]}</option>{/each}
      </select>
    {/if}
    <button class="quiet" aria-label={`Remove the filter on ${name}`} title="Remove this filter" onclick={onremove}>&#215;</button>
  </header>

  {#if error}
    <p class="error-text">{error}</p>
  {:else if !summary}
    <p class="muted">Reading the column…</p>
  {:else if filter.role !== 'discrete'}
    {#if ranged}
      <RangeSlider min={summary.min ?? 0} max={summary.max ?? 0} {low} {high} label={name} format={show} onchange={setRange} />
      <div class="bounds">
        <input type="text" aria-label={`${name} from`} value={show(low)} readonly={summary.timeOnly} onchange={(e) => typed('min', e.currentTarget.value)} />
        <span class="muted">to</span>
        <input type="text" aria-label={`${name} to`} value={show(high)} readonly={summary.timeOnly} onchange={(e) => typed('max', e.currentTarget.value)} />
      </div>
      {#if summary.missing > 0}
        <p class="muted">
          {formatCount(summary.missing)} {summary.missing === 1 ? 'row has' : 'rows have'} no value here; narrowing the range leaves {summary.missing === 1 ? 'it' : 'them'} out.
        </p>
      {/if}
    {:else}
      <p class="muted">No row has a {filter.role === 'time' ? 'date or time' : 'number'} in this column.</p>
    {/if}
  {:else}
    {#if (summary.categories.length ?? 0) > 8}
      <input type="search" placeholder="Find a value" aria-label={`Find a value of ${name}`} bind:value={search} />
    {/if}
    <div class="values" role="group" aria-label={`Values of ${name}`}>
      {#each listed as category (category.label)}
        <label>
          <input type="checkbox" checked={filter.values.includes(category.label)} onchange={(e) => toggle(category.label, e.currentTarget.checked)} />
          <span class="value" class:null={category.label === null} title={category.label ?? 'NULL'}>{category.label ?? NULL_LABEL}</span>
          <span class="muted count">{formatCount(category.rows)}</span>
        </label>
      {:else}
        <p class="muted">Nothing matches.</p>
      {/each}
    </div>
    <p class="muted">
      {filter.values.length === 0 ? 'Nothing ticked: every value is kept.' : `Only the ${filter.values.length} ticked ${filter.values.length === 1 ? 'value is' : 'values are'} kept.`}
      {#if filter.values.length > 0}<button class="link" onclick={() => (filter.values = [])}>Clear</button>{/if}
      {#if summary.overflow || summary.distinct > summary.categories.length}
        Only the {formatCount(summary.categories.length)} most frequent values are listed.
      {/if}
    </p>
  {/if}
</section>

<style>
  .filter {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px 8px;
    background: var(--bg);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  header {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .name {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  header select {
    padding: 1px 4px;
    font-size: 12px;
  }

  header button {
    padding: 0 6px;
  }

  .bounds {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .bounds input {
    flex: 1;
    min-width: 0;
    padding: 2px 5px;
    font-size: 12px;
  }

  input[type='search'] {
    padding: 2px 6px;
    font-size: 12px;
  }

  .values {
    max-height: 168px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 2px 4px;
  }

  .values label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 1px 0;
    cursor: pointer;
  }

  .value {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value.null {
    font-style: italic;
    color: var(--muted);
  }

  .count {
    font-variant-numeric: tabular-nums;
    font-size: 12px;
  }

  p {
    margin: 0;
    font-size: 12px;
  }

  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    font-size: 12px;
  }
</style>
