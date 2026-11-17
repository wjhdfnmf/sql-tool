<script lang="ts">
  // The graph builder's popups: a line across the chart, a trend line, and a name (title or axis).

  import { onMount, untrack } from 'svelte';
  import { LINE_ROLES, TREND_LABELS, formatNumber, formatTime, parseTime, type RefLine, type TrendKind } from '../lib/chart';
  import type { GraphDialogRequest, TrendSetup } from '../lib/graph.svelte';

  interface Props {
    request: GraphDialogRequest;
    onclose: () => void;
  }

  let { request, onclose }: Props = $props();

  let dialog: HTMLDialogElement;
  let problem = $state('');
  // What the popup was opened with. The fields start from it and are the user's from then on.
  const first = untrack(() => request);

  // ---- a line ----
  let line = $state<RefLine>(
    first.type === 'line' ? { ...first.line } : { id: 0, subject: '', axis: 'y', time: false, value: 0, style: 'dashed', color: 'red', name: '', role: 'none' },
  );
  const lineTime = $derived(request.type === 'line' && request.time[line.axis]);
  let valueText = $state(first.type === 'line' ? (first.time[first.line.axis] ? formatTime(first.line.value) : String(Number(first.line.value.toPrecision(6)))) : '');

  // ---- a trend line ----
  let trend = $state<TrendSetup>(
    first.type === 'trend' && first.setup ? { ...first.setup } : { kind: 'linear', degree: 2, window: 5, perGroup: false, showEquation: true },
  );
  const TREND_NOTES: Record<TrendKind | 'moving', string> = {
    linear: 'The straight line closest to the points: a steady rise or fall.',
    polynomial: 'A curve that can bend: once at degree 2, twice at degree 3, and so on. High degrees follow noise.',
    exponential: 'Growth or decay by a constant factor. Rows where Y is zero or less are left out.',
    logarithmic: 'A fast change that levels off. Rows where X is zero or less are left out.',
    power: 'Y in proportion to a power of X. Rows where either is zero or less are left out.',
    moving: 'Each point is the average of itself and the points before it, to smooth out the jumps.',
  };

  // ---- a name ----
  let text = $state(first.type === 'text' ? first.value : '');

  onMount(() => dialog.showModal());

  function apply(e: SubmitEvent) {
    e.preventDefault();
    if (request.type === 'line') {
      const value = lineTime ? parseTime(valueText) : Number(valueText.replaceAll(',', '').trim());
      if (value === null || valueText.trim() === '' || !Number.isFinite(value)) {
        problem = lineTime ? 'Enter a date, or a date and time, such as 2026-07-04 13:30.' : 'Enter a number.';
        return;
      }
      request.save({ ...line, value, name: line.name.trim(), role: line.axis === request.measured ? line.role : 'none' });
    } else if (request.type === 'trend') {
      request.save({
        ...trend,
        degree: Math.min(6, Math.max(2, Math.round(trend.degree) || 2)),
        window: Math.min(500, Math.max(2, Math.round(trend.window) || 5)),
        perGroup: trend.perGroup && request.perGroup,
      });
    } else {
      request.save(text.trim());
    }
    dialog.close();
  }

  function remove() {
    if (request.type !== 'text') request.remove?.();
    dialog.close();
  }
</script>

<dialog bind:this={dialog} onclose={onclose} aria-labelledby="graph-dialog-title">
  <form class="body" onsubmit={apply}>
    {#if request.type === 'line'}
      <h2 id="graph-dialog-title">{request.remove ? 'Line across the chart' : 'Add a line across the chart'}</h2>
      <label>
        <span>Axis</span>
        {#if request.axes.length > 1}
          <select bind:value={line.axis}>
            <option value="x">X: a vertical line</option>
            <option value="y">Y: a horizontal line</option>
          </select>
        {:else}
          <span class="fixed">{line.axis === 'x' ? 'X: a vertical line' : 'Y: a horizontal line'}</span>
        {/if}
      </label>
      <label>
        <span>Value</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" bind:value={valueText} autofocus oninput={() => (problem = '')} placeholder={lineTime ? '2026-07-04 13:30' : formatNumber(0)} />
      </label>
      <label>
        <span>Name</span>
        <input type="text" bind:value={line.name} maxlength="60" placeholder="Optional, shown beside the line" />
      </label>
      <label>
        <span>Line</span>
        <select bind:value={line.style}>
          <option value="solid">Solid</option>
          <option value="dashed">Dashed</option>
          <option value="dotted">Dotted</option>
        </select>
        <select bind:value={line.color} aria-label="Colour of the line">
          <option value="red">Red</option>
          <option value="green">Green</option>
          <option value="blue">Blue</option>
          <option value="grey">Grey</option>
        </select>
      </label>
      {#if request.measured === line.axis}
        <label>
          <span>Meaning</span>
          <select bind:value={line.role}>
            {#each Object.entries(LINE_ROLES) as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
        <p class="muted">Marking a line as a specification limit adds the capability figures (Cp, Cpk, Pp, Ppk, share outside) beside the chart.</p>
      {/if}
    {:else if request.type === 'trend'}
      <h2 id="graph-dialog-title">Trend line</h2>
      <label>
        <span>Shape</span>
        <!-- svelte-ignore a11y_autofocus -->
        <select bind:value={trend.kind} autofocus>
          {#each Object.entries(TREND_LABELS).filter(([kind]) => kind !== 'moving' || request.moving) as [value, label]}
            <option {value}>{label}</option>
          {/each}
        </select>
      </label>
      {#if trend.kind === 'polynomial'}
        <label><span>Degree</span><input type="number" min="2" max="6" bind:value={trend.degree} /></label>
      {:else if trend.kind === 'moving'}
        <label><span>Points averaged</span><input type="number" min="2" max="500" bind:value={trend.window} /></label>
      {/if}
      <p class="muted">{TREND_NOTES[trend.kind]}</p>
      {#if request.perGroup}
        <label class="check"><input type="checkbox" bind:checked={trend.perGroup} /> <span>A line for each colour group</span></label>
      {/if}
      <label class="check">
        <input type="checkbox" bind:checked={trend.showEquation} />
        <span>{trend.kind === 'moving' ? 'Say on the chart what the line is' : 'Write the equation and R² on the chart'}</span>
      </label>
    {:else}
      <h2 id="graph-dialog-title">{request.heading}</h2>
      <label>
        <span>{request.label}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="text" bind:value={text} maxlength="120" placeholder={request.placeholder} autofocus />
      </label>
      <p class="muted">Leave it empty for {request.placeholder === 'None' ? 'no title' : 'the automatic name'}.</p>
    {/if}

    {#if problem}<p class="error-text">{problem}</p>{/if}
    <div class="actions">
      {#if request.type !== 'text' && request.remove}
        <button type="button" class="danger" onclick={remove}>Remove</button>
      {/if}
      <span class="spacer"></span>
      <button type="button" onclick={() => dialog.close()}>Cancel</button>
      <button type="submit" class="primary">{request.type === 'line' && !request.remove ? 'Add line' : 'Apply'}</button>
    </div>
  </form>
</dialog>

<style>
  .body {
    width: 400px;
    max-width: 100%;
    padding: 16px 18px 14px;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 15px;
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  label > span:first-child {
    width: 104px;
    flex: none;
    color: var(--muted);
  }

  label.check > span {
    width: auto;
    color: var(--text);
  }

  label input[type='text'],
  label input[type='number'],
  label select {
    flex: 1;
    min-width: 0;
  }

  .fixed {
    color: var(--text);
  }

  p {
    margin: 0;
    font-size: 12px;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }

  .spacer {
    flex: 1;
  }
</style>
