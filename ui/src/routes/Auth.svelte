<script lang="ts">
  import { session } from '../lib/stores/session.svelte';

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
  <h1>Crosswire</h1>
  <p>Paste the bootstrap URL or token printed by the server to sign in.</p>
  <form onsubmit={submit}>
    <input
      data-testid="token-input"
      type="password"
      autocomplete="off"
      spellcheck="false"
      placeholder="http://localhost:PORT/#bootstrap=..."
      aria-label="Bootstrap URL or token"
      bind:value
    />
    <button data-testid="token-submit" type="submit" disabled={busy || value.trim() === ''}>
      Sign in
    </button>
  </form>
  {#if session.error}
    <p data-testid="auth-error" role="alert">{session.error}</p>
  {/if}
</main>
