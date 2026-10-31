<script lang="ts">
  import { session } from '../lib/stores/session.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Icon from '../lib/ui/Icon.svelte';

  let value = $state('');
  let busy = $state(false);

  async function submit(ev: SubmitEvent) {
    ev.preventDefault();
    busy = true;
    try {
      await session.submitToken(value);
    } finally {
      value = '';
      busy = false;
    }
  }
</script>

<main data-testid="auth-screen" class="auth">
  <div class="panel">
    <h1>Crosswire</h1>
    <p class="lede">
      Paste the bootstrap URL or token that <code>crosswire serve</code> printed. It works once; run
      <code>crosswire serve</code> again for a new one.
    </p>
    <form onsubmit={submit}>
      <Input
        data-testid="token-input"
        type="password"
        autocomplete="off"
        spellcheck="false"
        mono
        placeholder="http://127.0.0.1:8081/#bootstrap=…"
        aria-label="Bootstrap URL or token"
        bind:value
      />
      <Button
        variant="primary"
        type="submit"
        data-testid="token-submit"
        disabled={busy || value.trim() === ''}>Sign in</Button
      >
    </form>
    {#if session.error}
      <p class="error" data-testid="auth-error" role="alert">
        <Icon name="warning" size={14} /><span>{session.error}</span>
      </p>
    {/if}
  </div>
</main>

<style>
  .auth {
    display: grid;
    place-items: center;
    min-height: 100vh;
    min-height: 100dvh;
    padding: var(--s-4);
    background: var(--bg);
  }
  .panel {
    width: min(30rem, 100%);
    padding: var(--s-6);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  h1 {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .lede {
    margin: var(--s-2) 0 var(--s-4);
    font-size: var(--fs-md);
    line-height: 1.5;
    color: var(--fg-muted);
  }
  code {
    font-family: var(--font-mono);
    color: var(--fg);
  }
  form {
    display: flex;
    gap: var(--s-2);
  }
  form :global(.input) {
    flex: 1 1 auto;
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    margin: var(--s-3) 0 0;
    padding: var(--s-2) var(--s-3);
    font-size: var(--fs-sm);
    color: var(--danger-fg);
    background: var(--danger-bg);
    border-radius: var(--r-1);
  }
  .error :global(.icon) {
    flex: none;
    margin-top: 1px;
  }
</style>
