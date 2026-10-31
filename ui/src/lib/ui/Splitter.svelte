<script lang="ts">
  // Resizable two-pane layout (U1 spec D2).
  //
  // CSP note: the production policy has no 'unsafe-inline' for styles, so no
  // style="" attribute or style: directive is used. The split is a single
  // numeric CSS custom property written through the CSSOM
  // (element.style.setProperty), which CSP does not govern; the grid tracks
  // that consume it are declared in the stylesheet below.
  import { untrack, type Snippet } from 'svelte';
  import { HANDLE_PX, clampRatio, loadRatio, saveRatio } from './splitter';

  let {
    direction = 'row',
    min = [200, 240],
    storageKey,
    initial = 0.5,
    label = 'Resize panes',
    a,
    b,
  }: {
    /** 'row': panes side by side (vertical handle); 'column': stacked. */
    direction?: 'row' | 'column';
    /** Minimum px of pane A and pane B. */
    min?: [number, number];
    /** Persist the ratio under localStorage["crosswire.split.<key>"]. */
    storageKey?: string;
    initial?: number;
    label?: string;
    a: Snippet;
    b: Snippet;
  } = $props();

  let root = $state<HTMLElement>();
  let ratio = $state(0.5);
  let dragging = $state(false);

  const STEP = 0.02;
  const BIG_STEP = 0.1;

  function size(): number {
    if (!root) return 0;
    const r = root.getBoundingClientRect();
    return direction === 'row' ? r.width : r.height;
  }

  function setRatio(next: number, persist: boolean) {
    ratio = clampRatio(next, size(), min);
    if (persist && storageKey) saveRatio(storageKey, ratio);
  }

  // Initial ratio: persisted value when present, clamped to the current size.
  $effect.pre(() => {
    const stored = storageKey ? loadRatio(storageKey) : null;
    ratio = stored ?? initial;
  });

  $effect(() => {
    if (!root) return;
    untrack(() => setRatio(ratio, false));
    // Re-clamp when the container is resized (window resize, sibling layout).
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => setRatio(untrack(() => ratio), false));
    ro.observe(root);
    return () => ro.disconnect();
  });

  $effect(() => {
    root?.style.setProperty('--split', ratio.toFixed(4));
  });

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    dragging = true;
  }

  function onpointermove(e: PointerEvent) {
    if (!dragging || !root) return;
    const r = root.getBoundingClientRect();
    const pos = direction === 'row' ? e.clientX - r.left : e.clientY - r.top;
    const total = direction === 'row' ? r.width : r.height;
    if (total > HANDLE_PX) setRatio((pos - HANDLE_PX / 2) / (total - HANDLE_PX), false);
  }

  function onpointerup(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
    setRatio(ratio, true);
  }

  function onkeydown(e: KeyboardEvent) {
    const dec = direction === 'row' ? 'ArrowLeft' : 'ArrowUp';
    const inc = direction === 'row' ? 'ArrowRight' : 'ArrowDown';
    const step = e.shiftKey ? BIG_STEP : STEP;
    let next: number | null = null;
    if (e.key === dec) next = ratio - step;
    else if (e.key === inc) next = ratio + step;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = 1;
    if (next === null) return;
    e.preventDefault();
    setRatio(next, true);
  }
</script>

<div class="splitter {direction}" class:dragging bind:this={root}>
  <div class="pane pane-a">{@render a()}</div>
  <!-- A focusable separator is an interactive widget (WAI-ARIA window splitter). -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="handle"
    role="separator"
    tabindex="0"
    aria-label={label}
    aria-orientation={direction === 'row' ? 'vertical' : 'horizontal'}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(ratio * 100)}
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}
    {onkeydown}
  ></div>
  <div class="pane pane-b">{@render b()}</div>
</div>

<style>
  .splitter {
    --handle: 5px;
    display: grid;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .splitter.row {
    grid-template-columns:
      minmax(0, calc((100% - var(--handle)) * var(--split, 0.5)))
      var(--handle)
      minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
  }
  .splitter.column {
    grid-template-rows:
      minmax(0, calc((100% - var(--handle)) * var(--split, 0.5)))
      var(--handle)
      minmax(0, 1fr);
    grid-template-columns: minmax(0, 1fr);
  }
  .pane {
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }
  .handle {
    position: relative;
    background: transparent;
    touch-action: none;
  }
  .row > .handle {
    cursor: col-resize;
  }
  .column > .handle {
    cursor: row-resize;
  }
  /* The visible hairline; the 5 px handle is the hit area. */
  .handle::after {
    content: '';
    position: absolute;
    background: var(--border);
  }
  .row > .handle::after {
    top: 0;
    bottom: 0;
    left: 2px;
    width: 1px;
  }
  .column > .handle::after {
    left: 0;
    right: 0;
    top: 2px;
    height: 1px;
  }
  .handle:hover::after,
  .dragging > .handle::after,
  .handle:focus-visible::after {
    background: var(--accent);
  }
  .row > .handle:hover::after,
  .row > .handle:focus-visible::after,
  .row.dragging > .handle::after {
    left: 1px;
    width: 3px;
  }
  .column > .handle:hover::after,
  .column > .handle:focus-visible::after,
  .column.dragging > .handle::after {
    top: 1px;
    height: 3px;
  }
  .handle:focus-visible {
    outline: none;
  }
  .dragging {
    user-select: none;
  }
</style>
