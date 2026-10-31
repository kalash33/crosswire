<script lang="ts">
  import { scope } from '../stores/scope.svelte';
  import { describeRule } from './scope-rows';

  let text = $state('');
</script>

<div class="import" data-testid="scope-import">
  <textarea
    data-testid="import-text"
    aria-label="program scope list"
    rows="6"
    placeholder="Paste a program scope list: hosts, *.wildcards, URLs, CIDR blocks"
    bind:value={text}
  ></textarea>
  <div>
    <button type="button" data-testid="import-run" disabled={text.trim() === ''} onclick={() => void scope.importList(text)}
      >Import (preview)</button
    >
  </div>

  {#if scope.preview}
    <section data-testid="import-preview">
      <p>Preview (not saved yet). Applying REPLACES the current scope.</p>
      <h4>Include</h4>
      <ul>
        {#each scope.preview.include as r, i (i)}<li data-testid="preview-rule">{describeRule(r)}</li>{/each}
      </ul>
      <h4>Exclude</h4>
      <ul>
        {#each scope.preview.exclude as r, i (i)}<li data-testid="preview-rule">{describeRule(r)}</li>{/each}
      </ul>
      <button type="button" data-testid="import-apply" disabled={!scope.loaded} onclick={() => void scope.applyPreview()}>Apply to scope</button>
      <button type="button" data-testid="import-discard" onclick={() => scope.discardPreview()}>Discard</button>
    </section>
  {/if}
</div>

<style>
  .import {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  textarea {
    font-family: var(--font-mono);
    width: 100%;
    box-sizing: border-box;
  }
  h4 {
    margin: 0.3rem 0 0.1rem;
    font-size: 0.8rem;
  }
  ul {
    margin: 0;
    padding-left: 1.2rem;
    font-size: 0.85rem;
  }
</style>
