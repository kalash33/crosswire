<script lang="ts">
  import type { RulesDto } from '../api/intercept';
  import { intercept } from '../stores/intercept.svelte';

  // Local draft of the rules; Save PUTs it. Rule strings are attacker-influenced
  // text: bound to <input> values only, never injected as markup.
  let draft = $state<RulesDto>({ ...intercept.rules, skip_extensions: [...intercept.rules.skip_extensions] });
  let extText = $state(intercept.rules.skip_extensions.join(', '));
  let saved = $state(false);

  // Re-seed when the server's rules change underneath us (e.g. after refresh).
  let lastSeen = JSON.stringify(intercept.rules);
  $effect(() => {
    const now = JSON.stringify(intercept.rules);
    if (now !== lastSeen) {
      lastSeen = now;
      draft = { ...intercept.rules, skip_extensions: [...intercept.rules.skip_extensions] };
      extText = draft.skip_extensions.join(', ');
    }
  });

  async function save() {
    const skip_extensions = extText
      .split(/[,\s]+/)
      .map((s) => s.trim().replace(/^\./, ''))
      .filter((s) => s !== '');
    await intercept.putRules({ ...draft, skip_extensions });
    saved = intercept.error === null;
  }
</script>

<form
  class="rules"
  data-testid="rules-editor"
  onsubmit={(e) => {
    e.preventDefault();
    void save();
  }}
>
  <label>
    Skip extensions
    <input data-testid="rules-skip-ext" bind:value={extText} placeholder="png, jpg, css" />
  </label>
  <label class="inline">
    <input type="checkbox" data-testid="rules-skip-oos" bind:checked={draft.skip_out_of_scope} />
    Skip out-of-scope
  </label>
  <label>
    On overflow
    <select data-testid="rules-overflow" bind:value={draft.overflow}>
      <option value="forward-newest">forward newest</option>
      <option value="forward-oldest">forward oldest</option>
    </select>
  </label>
  <label>
    On timeout
    <select data-testid="rules-timeout" bind:value={draft.on_timeout}>
      <option value="forward">forward</option>
      <option value="respond-504">respond 504</option>
    </select>
  </label>
  <button type="submit" data-testid="rules-save">Save rules</button>
  {#if saved}<span data-testid="rules-saved">saved</span>{/if}
</form>

<style>
  .rules {
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
    align-items: end;
    font-size: 0.85rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  label.inline {
    flex-direction: row;
    align-items: center;
    gap: 0.35rem;
  }
</style>
