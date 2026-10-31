<script lang="ts">
  import { scope } from '../stores/scope.svelte';
  import { describeRule } from './scope-rows';
  import Button from '../ui/Button.svelte';
  import { toasts } from '../ui/toasts.svelte';

  let text = $state('');

  async function run() {
    const ok = await scope.importList(text);
    if (!ok) toasts.push({ kind: 'error', text: scope.error ?? 'Import failed' });
  }
  async function apply() {
    const ok = await scope.applyPreview();
    if (ok) toasts.push({ kind: 'success', text: 'Imported scope applied' });
    else if (scope.error) toasts.push({ kind: 'error', text: scope.error });
  }
</script>

<section class="card import" data-testid="scope-import">
  <h2>Import program list</h2>
  <textarea
    data-testid="import-text"
    aria-label="program scope list"
    rows="6"
    placeholder="Paste a program scope list: hosts, *.wildcards, URLs, CIDR blocks"
    bind:value={text}
  ></textarea>
  <div>
    <Button data-testid="import-run" disabled={text.trim() === ''} onclick={() => void run()}
      >Import (preview)</Button
    >
  </div>

  {#if scope.preview}
    <section class="preview" data-testid="import-preview">
      <p>Preview (not saved yet). Applying REPLACES the current scope.</p>
      <div class="cols">
        <div>
          <h3>Include</h3>
          <ul>
            {#each scope.preview.include as r, i (i)}<li data-testid="preview-rule">{describeRule(r)}</li>{/each}
          </ul>
        </div>
        <div>
          <h3>Exclude</h3>
          <ul>
            {#each scope.preview.exclude as r, i (i)}<li data-testid="preview-rule">{describeRule(r)}</li>{/each}
          </ul>
        </div>
      </div>
      <div class="actions">
        <Button variant="primary" data-testid="import-apply" disabled={!scope.loaded} onclick={() => void apply()}
          >Apply to scope</Button
        >
        <Button variant="ghost" data-testid="import-discard" onclick={() => scope.discardPreview()}>Discard</Button>
      </div>
    </section>
  {/if}
</section>

<style>
  .card {
    padding: var(--s-3) var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-md);
    font-weight: 600;
  }
  textarea {
    font-family: var(--font-mono);
    width: 100%;
    box-sizing: border-box;
  }
  .preview {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-1);
  }
  .preview p {
    margin: 0;
    color: var(--warn-fg);
  }
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: var(--s-3);
  }
  h3 {
    margin: 0 0 var(--s-1);
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  ul {
    margin: 0;
    padding-left: var(--s-4);
    font-size: var(--fs-md);
    font-family: var(--font-mono);
  }
  .actions {
    display: flex;
    gap: var(--s-2);
  }
</style>
