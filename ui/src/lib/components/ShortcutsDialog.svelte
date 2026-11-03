<script lang="ts">
  // Keyboard cheat sheet. A native modal <dialog>: showModal() makes the rest
  // of the page inert (focus is trapped inside) and Escape fires `cancel`,
  // which closes it. Focus goes back to whatever opened it.
  import { onMount } from 'svelte';
  import { keymap, formatChord, SCOPES, SCOPE_LABELS } from '../keymap';
  import { chordLabel } from '../keymap-hints.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Kbd from '../ui/Kbd.svelte';

  let { onClose }: { onClose: () => void } = $props();

  let dlg: HTMLDialogElement;
  // Re-read whenever a chord changes (chordLabel is reactive; the sheet is
  // modal, so remaps happen only while it is closed, but stay correct anyway).
  const groups = $derived.by(() => {
    void chordLabel('find');
    const all = keymap.bindings();
    return SCOPES.map((scope) => ({
      scope,
      label: SCOPE_LABELS[scope],
      rows: all.filter((b) => b.scope === scope),
    })).filter((g) => g.rows.length > 0);
  });

  // History's list-widget keys: ARIA grid conventions, not remappable.
  const LIST_KEYS = [
    { label: 'Next / previous row', keys: ['J', 'K', 'Down', 'Up'] },
    { label: 'Open the row', keys: ['Enter', 'Space'] },
    { label: 'First / last row', keys: ['Home', 'End'] },
    { label: 'Page up / down', keys: ['Page Up', 'Page Down'] },
    { label: 'Close the detail', keys: ['Esc'] },
  ];

  onMount(() => {
    const opener = document.activeElement;
    dlg.showModal();
    return () => {
      if (opener instanceof HTMLElement) opener.focus();
    };
  });
</script>

<dialog
  bind:this={dlg}
  class="sheet"
  data-testid="shortcuts-dialog"
  aria-labelledby="shortcuts-title"
  onclose={onClose}
>
  <div class="head">
    <h2 id="shortcuts-title">Keyboard shortcuts</h2>
    <IconButton icon="close" size="sm" label="Close" onclick={() => dlg.close()} />
  </div>
  {#each groups as g (g.scope)}
    <section data-testid="shortcut-group-{g.scope}">
      <h3>{g.label}</h3>
      <table>
        <tbody>
          {#each g.rows as r (r.action)}
            <tr data-testid="shortcut-row" data-action={r.action}>
              <td class="label">{r.label}</td>
              <td class="chord">
                {#if r.chord}<Kbd keys={formatChord(r.chord)} />{:else}<span class="unset">Not set</span>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/each}
  <section data-testid="shortcut-group-history-list">
    <h3>History list</h3>
    <table>
      <tbody>
        {#each LIST_KEYS as k (k.label)}
          <tr data-testid="shortcut-fixed-row">
            <td class="label">{k.label}</td>
            <td class="chord">{#each k.keys as key (key)}<Kbd keys={key} /> {/each}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>
  <p class="foot">Shortcuts without Ctrl are ignored while you type in a field.</p>
</dialog>

<style>
  .sheet {
    width: min(30rem, calc(100vw - 2 * var(--s-4)));
    max-height: calc(100vh - 6rem);
    overflow-y: auto;
    padding: var(--s-3) var(--s-4) var(--s-4);
    color: var(--fg);
    background: var(--bg-overlay);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-2);
    box-shadow: var(--shadow-pop);
  }
  .sheet::backdrop {
    background: var(--backdrop);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--s-2);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  h3 {
    margin: var(--s-3) 0 var(--s-1);
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg-muted);
  }
  .unset {
    color: var(--fg-muted);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-md);
  }
  td {
    padding: 6px 0;
    border-bottom: 1px solid var(--border-faint);
  }
  tr:last-child td {
    border-bottom: none;
  }
  .label {
    color: var(--fg);
  }
  .chord {
    text-align: right;
  }
  .foot {
    margin: var(--s-3) 0 0;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
</style>
