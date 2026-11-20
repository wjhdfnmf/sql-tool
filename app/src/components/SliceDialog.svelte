<script lang="ts">
  // Running one long query as many short ones: the time range is cut into
  // slices, the query is run for each, and the rows are put end to end.

  import { onMount, untrack } from 'svelte';
  import { backend } from '../lib/backend';
  import { asUiError, formatCount, type SlicePlan } from '../lib/types';

  export interface SliceSetup {
    start: string;
    end: string;
    every: number;
    unit: 'minutes' | 'hours' | 'days';
  }

  interface Props {
    /** The query, with {start} and {end} where its time range goes. */
    sql: string;
    /** What was used the last time, if anything. */
    last: SliceSetup | null;
    onrun: (setup: SliceSetup, stepSecs: number) => void;
    onclose: () => void;
  }

  let { sql, last, onrun, onclose }: Props = $props();

  const UNIT_SECONDS = { minutes: 60, hours: 3600, days: 86_400 };
  const pad = (n: number) => String(n).padStart(2, '0');
  /** A time as it is typed here: this computer's clock, to the minute. */
  const stamp = (at: Date) => `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;

  let dialog: HTMLDialogElement;
  // The last week, an hour at a time, unless something else was used before.
  let setup = $state<SliceSetup>(untrack(() => last) ?? { start: stamp(new Date(Date.now() - 7 * 86_400_000)), end: stamp(new Date()), every: 1, unit: 'hours' });
  let plan = $state.raw<SlicePlan | null>(null);
  let problem = $state('');

  const stepSecs = $derived(Math.max(1, Math.round(Number(setup.every) || 0)) * UNIT_SECONDS[setup.unit]);
  /** BETWEEN takes in both ends, so a row exactly on a boundary would come back from two slices. */
  const overlaps = $derived(/between\s+'?\{(start|0)\}/i.test(sql));

  onMount(() => dialog.showModal());

  $effect(() => {
    const [from, to, step] = [setup.start, setup.end, stepSecs];
    let stale = false;
    const timer = setTimeout(() => {
      backend
        .slicePlan(sql, from, to, step)
        .then((found) => {
          if (stale) return;
          plan = found;
          problem = '';
        })
        .catch((e) => {
          if (stale) return;
          plan = null;
          problem = asUiError(e).message;
        });
    }, 150);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  });

  function run(e: SubmitEvent) {
    e.preventDefault();
    if (!plan) return;
    onrun($state.snapshot(setup), stepSecs);
    dialog.close();
  }
</script>

<dialog bind:this={dialog} onclose={onclose} aria-labelledby="slice-title">
  <form class="body" onsubmit={run}>
    <h2 id="slice-title">Run in time slices</h2>
    <p class="muted">
      A query over a long stretch of time is run as many short ones, one for each slice of the range, and the rows are put end to end as
      one result. No single run is long, so none times out. The query says where its range goes with <code>{'{start}'}</code> and
      <code>{'{end}'}</code>:
    </p>
    <pre class="example">WHERE completed &gt;= {'{start}'} AND completed &lt; {'{end}'}</pre>

    <div class="fields">
      <label>
        <span>From</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" bind:value={setup.start} placeholder="2026-07-01 00:00" autofocus />
      </label>
      <label>
        <span>To</span>
        <input type="text" bind:value={setup.end} placeholder="2026-07-08 00:00" />
      </label>
      <label>
        <span>Each slice</span>
        <input type="number" min="1" max="100000" bind:value={setup.every} />
        <select bind:value={setup.unit} aria-label="Unit of a slice">
          <option value="minutes">minutes</option>
          <option value="hours">hours</option>
          <option value="days">days</option>
        </select>
      </label>
    </div>
    <p class="muted">Times are sent to the server as typed, with no time zone: use the zone the column is stored in.</p>

    {#if overlaps}
      <p class="warning-box">
        <code>BETWEEN</code> takes in both ends, so a row exactly on a boundary would come back from two slices. Use
        <code>&gt;= {'{start}'} AND &lt; {'{end}'}</code> to get each row once.
      </p>
    {/if}

    {#if problem}
      <p class="error-text" role="alert">{problem}</p>
    {:else if plan}
      <p>
        <strong>{formatCount(plan.slices)} {plan.slices === 1 ? 'slice' : 'slices'}</strong>{plan.threads > 1 ? `, ${plan.threads} at a time` : ', one after another'}.
        The first:
      </p>
      <pre class="sql">{plan.first}</pre>
    {:else}
      <p class="muted">Working out the slices…</p>
    {/if}

    <div class="actions">
      <span class="spacer"></span>
      <button type="button" onclick={() => dialog.close()}>Cancel</button>
      <button type="submit" class="primary" disabled={!plan}>Run {plan ? formatCount(plan.slices) : ''} {plan?.slices === 1 ? 'slice' : 'slices'}</button>
    </div>
  </form>
</dialog>

<style>
  .body {
    width: 560px;
    max-width: 100%;
    padding: 16px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
  }

  p {
    margin: 0;
  }

  pre {
    margin: 0;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .sql {
    max-height: 132px;
    overflow-y: auto;
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  label > span:first-child {
    width: 84px;
    flex: none;
    color: var(--muted);
  }

  label input[type='text'] {
    width: 190px;
  }

  label input[type='number'] {
    width: 90px;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }

  .spacer {
    flex: 1;
  }
</style>
