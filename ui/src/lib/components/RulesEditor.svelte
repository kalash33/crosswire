<script lang="ts">
  import type { RulesDto } from '../api/intercept';
  import { intercept } from '../stores/intercept.svelte';
  import Button from '../ui/Button.svelte';
  import Input from '../ui/Input.svelte';
  import Select from '../ui/Select.svelte';
  import { toasts } from '../ui/toasts.svelte';

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
    if (saved) toasts.push({ kind: 'success', text: 'Intercept rules saved' });
    else toasts.push({ kind: 'error', text: intercept.error ?? 'Could not save rules' });
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
    <Input data-testid="rules-skip-ext" bind:value={extText} placeholder="png, jpg, css" />
  </label>
  <label class="inline">
    <input type="checkbox" data-testid="rules-skip-oos" bind:checked={draft.skip_out_of_scope} />
    Skip out-of-scope
  </label>
  <label>
    On overflow
    <Select data-testid="rules-overflow" bind:value={draft.overflow}>
      <option value="forward-newest">forward newest</option>
      <option value="forward-oldest">forward oldest</option>
    </Select>
  </label>
  <label>
    On timeout
    <Select data-testid="rules-timeout" bind:value={draft.on_timeout}>
      <option value="forward">forward</option>
      <option value="respond-504">respond 504</option>
    </Select>
  </label>
  <div class="foot">
    <Button type="submit" variant="primary" data-testid="rules-save">Save rules</Button>
    {#if saved}<span class="saved" data-testid="rules-saved">saved</span>{/if}
  </div>
</form>

<style>
  .rules {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    font-size: var(--fs-md);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    color: var(--fg-muted);
  }
  label.inline {
    flex-direction: row;
    align-items: center;
    gap: var(--s-2);
    color: var(--fg);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .saved {
    color: var(--ok-fg);
    font-size: var(--fs-sm);
  }
</style>
