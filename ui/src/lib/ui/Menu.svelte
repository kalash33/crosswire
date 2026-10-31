<script lang="ts">
  // Context / overflow menu (U1 spec D2). Items are <button>s; labels are text.
  //
  // Keyboard: ArrowUp/ArrowDown (wrapping), Home/End, Enter/Space selects,
  // Escape and Tab close. A pointerdown outside the menu (and its anchor)
  // closes it. Focus returns to the anchor element when the menu closes.
  //
  // CSP note: the position is written as two CSS custom properties through the
  // CSSOM (element.style.setProperty), which the style-src policy does not
  // govern; no style="" attribute or style: directive is emitted.
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';
  import type { IconName } from './icons';

  export interface MenuItem {
    label: string;
    icon?: IconName;
    kbd?: string;
    danger?: boolean;
    disabled?: boolean;
    testid?: string;
    onselect: () => void;
  }

  let {
    items,
    open,
    anchor,
    onclose,
    label = 'Actions',
    testid = 'menu',
  }: {
    items: readonly MenuItem[];
    open: boolean;
    /** Element to drop below, or a viewport point (context menu). */
    anchor: HTMLElement | { x: number; y: number } | null | undefined;
    onclose: () => void;
    label?: string;
    testid?: string;
  } = $props();

  let menu = $state<HTMLElement>();

  const EDGE = 4;

  function buttons(): HTMLButtonElement[] {
    return menu ? Array.from(menu.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]:not(:disabled)')) : [];
  }

  function focusAt(i: number) {
    const bs = buttons();
    if (bs.length === 0) return;
    bs[((i % bs.length) + bs.length) % bs.length].focus();
  }

  function current(): number {
    return buttons().indexOf(document.activeElement as HTMLButtonElement);
  }

  function restoreFocus() {
    if (anchor instanceof HTMLElement && anchor.isConnected) anchor.focus();
  }

  function close() {
    onclose();
    restoreFocus();
  }

  function select(item: MenuItem) {
    if (item.disabled) return;
    onclose();
    restoreFocus();
    item.onselect();
  }

  function position() {
    if (!menu || !anchor) return;
    let x: number;
    let y: number;
    if (anchor instanceof HTMLElement) {
      const r = anchor.getBoundingClientRect();
      x = r.left;
      y = r.bottom + 2;
    } else {
      x = anchor.x;
      y = anchor.y;
    }
    const m = menu.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    if (x + m.width > vw - EDGE) x = Math.max(EDGE, vw - EDGE - m.width);
    if (y + m.height > vh - EDGE) {
      // Flip above an element anchor; otherwise slide up into view.
      y = anchor instanceof HTMLElement ? anchor.getBoundingClientRect().top - 2 - m.height : vh - EDGE - m.height;
      y = Math.max(EDGE, y);
    }
    menu.style.setProperty('--menu-x', `${Math.round(x)}px`);
    menu.style.setProperty('--menu-y', `${Math.round(y)}px`);
  }

  $effect(() => {
    if (!open || !menu) return;
    position();
    focusAt(0);

    const onDown = (e: Event) => {
      const t = e.target as Node | null;
      if (t && menu?.contains(t)) return;
      if (t && anchor instanceof HTMLElement && anchor.contains(t)) return;
      onclose();
    };
    const onResize = () => onclose();
    document.addEventListener('pointerdown', onDown, true);
    window.addEventListener('resize', onResize);
    window.addEventListener('blur', onResize);
    return () => {
      document.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('resize', onResize);
      window.removeEventListener('blur', onResize);
    };
  });

  function onkeydown(e: KeyboardEvent) {
    // Keep menu keys away from the global keymap dispatch.
    e.stopPropagation();
    const i = current();
    switch (e.key) {
      case 'ArrowDown':
        focusAt(i + 1);
        break;
      case 'ArrowUp':
        focusAt(i < 0 ? -1 : i - 1);
        break;
      case 'Home':
        focusAt(0);
        break;
      case 'End':
        focusAt(-1);
        break;
      case 'Enter':
      case ' ': {
        const enabled = items.filter((it) => !it.disabled);
        const item = enabled[i < 0 ? 0 : i];
        if (item) select(item);
        break;
      }
      case 'Escape':
        close();
        break;
      case 'Tab':
        onclose();
        return;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

{#if open}
  <div class="menu" role="menu" aria-label={label} tabindex="-1" data-testid={testid} bind:this={menu} {onkeydown}>
    {#each items as item, i (i)}
      <button
        type="button"
        role="menuitem"
        tabindex="-1"
        class="item"
        class:danger={item.danger}
        disabled={item.disabled}
        data-testid={item.testid}
        onclick={() => select(item)}
      >
        <span class="ic">{#if item.icon}<Icon name={item.icon} size={14} />{/if}</span>
        <span class="lbl">{item.label}</span>
        {#if item.kbd}<Kbd keys={item.kbd} />{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    left: var(--menu-x, 0);
    top: var(--menu-y, 0);
    z-index: var(--z-pop);
    display: flex;
    flex-direction: column;
    min-width: 200px;
    max-width: 360px;
    max-height: calc(100vh - 8px);
    overflow-y: auto;
    padding: var(--s-1);
    background: var(--bg-overlay);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-2);
    box-shadow: var(--shadow-pop);
  }
  .menu:focus {
    outline: none;
  }
  .item {
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-2);
    height: 28px;
    min-height: 0;
    padding: 0 var(--s-2);
    font-size: var(--fs-md);
    text-align: left;
    color: var(--fg);
    background: transparent;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .item:hover:not(:disabled),
  .item:focus-visible {
    background: var(--bg-hover);
    outline: none;
  }
  .item:focus-visible {
    box-shadow: inset 0 0 0 1px var(--focus);
  }
  .item.danger {
    color: var(--danger);
  }
  .ic {
    display: inline-flex;
    color: var(--fg-muted);
  }
  .danger .ic {
    color: inherit;
  }
  .lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
