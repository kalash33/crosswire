<script lang="ts">
  // Keyboard cheat sheet. A native modal <dialog>: showModal() makes the rest
  // of the page inert (focus is trapped inside) and Escape fires `cancel`,
  // which closes it. Focus goes back to whatever opened it.
  import { onMount } from 'svelte';
  import { keymap, formatChord } from '../keymap';
  import IconButton from '../ui/IconButton.svelte';
  import Kbd from '../ui/Kbd.svelte';

  let { onClose }: { onClose: () => void } = $props();

  let dlg: HTMLDialogElement;
  const rows = keymap.bindings();

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
  <table>
    <tbody>
      {#each rows as r (r.action)}
        <tr data-testid="shortcut-row" data-action={r.action}>
          <td class="label">{r.label}</td>
          <td class="chord"><Kbd keys={formatChord(r.chord)} /></td>
        </tr>
      {/each}
    </tbody>
  </table>
  <p class="foot">Shortcuts without Ctrl are ignored while you type in a field.</p>
</dialog>

<style>
  .sheet {
    width: min(30rem, calc(100vw - 2 * var(--s-4)));
    max-height: calc(100vh - 6rem);
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
