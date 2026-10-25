<script lang="ts">
  import { onMount } from 'svelte';
  import ByteEditor from '../lib/editor/ByteEditor.svelte';
  import HeldList from '../lib/components/HeldList.svelte';
  import RulesEditor from '../lib/components/RulesEditor.svelte';
  import { intercept, splitRequest, concatBytes } from '../lib/stores/intercept.svelte';
  import { repeater } from '../lib/stores/repeater.svelte';
  import { router } from '../lib/router.svelte';
  import { keymap } from '../lib/keymap';

  onMount(() => {
    intercept.start();
    void intercept.refresh(); // a revisit must not show a stale list
    // Burp chords for the focused held item: forward (Ctrl+F), drop (Ctrl+D),
    // send-to-repeater (Ctrl+R). Registered while mounted; torn down on unmount.
    const offs = [
      keymap.register('forward', () => void forward()),
      keymap.register('drop', () => {
        if (focus) void intercept.drop(focus.id);
      }),
      keymap.register('send-to-repeater', () => toRepeater()),
    ];
    return () => offs.forEach((off) => off());
  });

  // The oldest held request is the focus default; the user may pick another.
  let pickedId = $state<string | null>(null);
  const focus = $derived(intercept.held.find((h) => h.id === pickedId) ?? intercept.held[0] ?? null);

  // The editor shows head||body as ONE raw buffer. `original` is what the
  // server holds; `bytes` is what the user is editing.
  let original = $state<{ head: Uint8Array; body: Uint8Array | null } | null>(null);
  let bytes = $state<Uint8Array>(new Uint8Array());
  let loadedFor = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let epoch = $state(0);

  $effect(() => {
    const id = focus?.id ?? null;
    if (id === loadedFor) return;
    loadedFor = id;
    original = null;
    loadError = null;
    if (id === null) return;
    void intercept
      .heldBytes(id)
      .then((b) => {
        if (loadedFor !== id) return; // focus moved on
        original = b;
        bytes = concatBytes(b.head, b.body);
        epoch += 1; // ByteEditor seeds once at mount: remount for the new doc
      })
      .catch((e: unknown) => {
        if (loadedFor === id) loadError = e instanceof Error ? e.message : String(e);
      });
  });

  function same(a: Uint8Array, b: Uint8Array): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }
  const originalBytes = $derived(original ? concatBytes(original.head, original.body) : null);
  const dirty = $derived(originalBytes !== null && !same(bytes, originalBytes));

  async function forward() {
    if (!focus) return;
    // Non-editable: always the ORIGINAL bytes (no body), never a truncation.
    if (!focus.editable || !dirty) return void (await intercept.forward(focus.id));
    const { head, body } = splitRequest(bytes);
    // An edited body REQUIRES the edited head; always send the head. Send the
    // body when there is one, or when the original had one (an emptied body).
    const hadBody = original?.body != null;
    await intercept.forward(focus.id, {
      head,
      body: body.length > 0 || hadBody ? body : null,
    });
  }

  function toRepeater() {
    if (!focus || !original) return;
    const raw = concatBytes(original.head, original.body);
    // `host` may carry a port; the rest is passed through as plain text.
    const m = /^(.*?)(?::(\d{1,5}))?$/.exec(focus.host);
    const port = m?.[2] ? Number(m[2]) : 80;
    repeater.newTab({
      conn: { host: m?.[1] ?? focus.host, port, tls: port === 443 },
      raw,
    });
    router.navigate('/repeater');
  }
</script>

<div class="intercept" data-testid="intercept-view">
  <div class="toolbar">
    <span data-testid="intercept-state">Intercept {intercept.enabled ? 'ON' : 'OFF'}</span>
    <button type="button" data-testid="forward-all" onclick={() => void intercept.forwardAll()}
      >Forward all</button
    >
  </div>

  {#if intercept.error}
    <p class="error" role="alert" data-testid="intercept-error">{intercept.error}</p>
  {/if}

  <HeldList held={intercept.held} selectedId={focus?.id ?? null} onselect={(id) => (pickedId = id)} />

  {#if focus}
    <section class="detail" data-testid="intercept-detail">
      <div class="actions">
        <button type="button" data-testid="forward" disabled={!original} onclick={() => void forward()}>
          Forward{dirty && focus.editable ? ' (edited)' : ''}
        </button>
        <button type="button" data-testid="drop" onclick={() => void intercept.drop(focus.id)}
          >Drop</button
        >
        <button type="button" data-testid="to-repeater" disabled={!original} onclick={toRepeater}
          >Send to repeater</button
        >
        {#if !focus.editable}
          <span class="badge" data-testid="not-editable-badge"
            >not editable (body over the hold limit; forwards original)</span
          >
        {/if}
      </div>
      {#if loadError}
        <p class="error" role="alert" data-testid="held-load-error">{loadError}</p>
      {:else if original}
        {#key `${focus.id}:${epoch}`}
          <ByteEditor bind:bytes readonly={!focus.editable} />
        {/key}
      {:else}
        <p class="placeholder">Loading held bytes…</p>
      {/if}
    </section>
  {/if}

  <details class="rules-box">
    <summary>Rules</summary>
    <RulesEditor />
  </details>
</div>

<style>
  .intercept {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .toolbar,
  .actions {
    display: flex;
    gap: 0.6rem;
    align-items: center;
  }
  .badge {
    font-size: 0.75rem;
    border: 1px solid #b9770e;
    color: #e0a040;
    border-radius: 3px;
    padding: 0.05rem 0.4rem;
  }
  .error {
    color: #e06060;
  }
</style>
