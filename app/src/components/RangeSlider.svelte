<script lang="ts">
  // A slider with two handles, for choosing a range of numbers or of time.

  interface Props {
    min: number;
    max: number;
    low: number;
    high: number;
    /** What the range is of, for screen readers: "amount", "time range". */
    label: string;
    /** A value as text, for screen readers. */
    format: (value: number) => string;
    /** Called as a handle moves. */
    onchange: (low: number, high: number) => void;
  }

  let { min, max, low, high, label, format, onchange }: Props = $props();

  let track: HTMLDivElement;
  const span = $derived(max - min);
  const usable = $derived(Number.isFinite(span) && span > 0);
  const share = (value: number) => (usable ? Math.min(1, Math.max(0, (value - min) / span)) : 0);
  const lowShare = $derived(share(low));
  const highShare = $derived(usable ? share(high) : 1);

  function valueAt(clientX: number): number {
    const box = track.getBoundingClientRect();
    return min + Math.min(1, Math.max(0, (clientX - box.left) / box.width)) * span;
  }

  function set(which: 'low' | 'high', value: number) {
    if (which === 'low') onchange(Math.min(Math.max(min, value), high), high);
    else onchange(low, Math.max(Math.min(max, value), low));
  }

  function drag(which: 'low' | 'high', e: PointerEvent) {
    if (!usable || e.button !== 0) return;
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => set(which, valueAt(ev.clientX));
    const stop = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', stop);
      handle.removeEventListener('pointercancel', stop);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', stop);
    handle.addEventListener('pointercancel', stop);
  }

  /** A click on the track moves whichever handle is nearer. */
  function jump(e: PointerEvent) {
    if (!usable || e.target !== e.currentTarget || e.button !== 0) return;
    const value = valueAt(e.clientX);
    set(Math.abs(value - low) <= Math.abs(value - high) ? 'low' : 'high', value);
  }

  function key(which: 'low' | 'high', e: KeyboardEvent) {
    if (!usable) return;
    const step = span * (e.shiftKey ? 0.1 : 0.01);
    const now = which === 'low' ? low : high;
    if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') set(which, now - step);
    else if (e.key === 'ArrowRight' || e.key === 'ArrowUp') set(which, now + step);
    else if (e.key === 'Home') set(which, min);
    else if (e.key === 'End') set(which, max);
    else return;
    e.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="range" class:off={!usable} bind:this={track} onpointerdown={jump}>
  <div class="fill" style:left={`${lowShare * 100}%`} style:width={`${Math.max(0, highShare - lowShare) * 100}%`}></div>
  {#each [['low', lowShare, low, 'from'], ['high', highShare, high, 'to']] as const as [which, at, value, word] (which)}
    <button
      class="handle"
      role="slider"
      aria-label={`${label}: ${word}`}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-valuetext={format(value)}
      disabled={!usable}
      style:left={`${at * 100}%`}
      onpointerdown={(e) => drag(which, e)}
      onkeydown={(e) => key(which, e)}
    ></button>
  {/each}
</div>

<style>
  .range {
    position: relative;
    height: 18px;
    margin: 0 7px;
    touch-action: none;
    cursor: pointer;
  }

  .range::before {
    content: '';
    position: absolute;
    left: -7px;
    right: -7px;
    top: 7px;
    height: 4px;
    border-radius: 2px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    box-sizing: border-box;
    pointer-events: none;
  }

  .fill {
    position: absolute;
    top: 7px;
    height: 4px;
    background: var(--accent);
    pointer-events: none;
  }

  .handle {
    position: absolute;
    top: 2px;
    width: 14px;
    height: 14px;
    margin-left: -7px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--accent);
    background: var(--bg);
    cursor: ew-resize;
  }

  .handle:hover:not(:disabled),
  .handle:focus-visible {
    background: var(--accent);
  }

  .off {
    cursor: default;
    opacity: 0.5;
  }

  .off .fill {
    display: none;
  }
</style>
