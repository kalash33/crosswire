<script lang="ts">
  import { onMount } from 'svelte';
  import ByteEditor from '../lib/editor/ByteEditor.svelte';
  import HeldList from '../lib/components/HeldList.svelte';
  import RulesEditor from '../lib/components/RulesEditor.svelte';
  import { intercept, splitRequest, concatBytes } from '../lib/stores/intercept.svelte';
  import { repeater } from '../lib/stores/repeater.svelte';
  import { router } from '../lib/router.svelte';
  import { keymap } from '../lib/keymap';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Toolbar from '../lib/ui/Toolbar.svelte';
  import { toasts } from '../lib/ui/toasts.svelte';

  // The store records a failure in `intercept.error` instead of throwing, so
  // the outcome of every action is read back from it.
  function report(done: string) {
    if (intercept.error) toasts.push({ kind: 'error', text: intercept.error });
    else toasts.push({ kind: 'success', text: done });
  }
  async function drop() {
    if (!focus) return;
    await intercept.drop(focus.id);
    report('Dropped');
  }
  async function forwardAll() {
    await intercept.forwardAll();
    report('Forwarded all held requests');
  }

  onMount(() => {
    intercept.start();
    void intercept.refresh(); // a revisit must not show a stale list
    // Burp chords for the focused held item: forward (Ctrl+F), drop (Ctrl+D),
    // send-to-repeater (Ctrl+R). Registered while mounted; torn down on unmount.
    const offs = [
      keymap.register('forward', () => void forward()),
      keymap.register('drop', () => void drop()),
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
    if (!focus.editable || !dirty) {
      await intercept.forward(focus.id);
      return report('Forwarded');
    }
    const { head, body } = splitRequest(bytes);
    // An edited body REQUIRES the edited head; always send the head. Send the
    // body when there is one, or when the original had one (an emptied body).
    const hadBody = original?.body != null;
    await intercept.forward(focus.id, {
      head,
      body: body.length > 0 || hadBody ? body : null,
    });
    report('Forwarded with your edits');
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
  <header class="top">
    <label class="switch">
      <input
        type="checkbox"
        role="switch"
        data-testid="intercept-switch"
        checked={intercept.enabled}
        onchange={(e) => void intercept.setEnabled(e.currentTarget.checked)}
      />
      <span class="track" aria-hidden="true"><span class="thumb"></span></span>
      <span class="state" data-testid="intercept-state">Intercept is {intercept.enabled ? 'ON' : 'OFF'}</span>
    </label>
    <Badge kind="neutral" value={`${intercept.held.length} held`} />
    <span class="spacer"></span>
    <Button
      data-testid="forward-all"
      icon="forward"
      disabled={intercept.held.length === 0}
      onclick={() => void forwardAll()}>Forward all</Button
    >
  </header>

  {#if intercept.error}
    <p class="alert" role="alert" data-testid="intercept-error">{intercept.error}</p>
  {/if}

  <div class="grid">
    <div class="work">
      <section class="card">
        <h2>Queue</h2>
        <HeldList
          held={intercept.held}
          enabled={intercept.enabled}
          selectedId={focus?.id ?? null}
          onselect={(id) => (pickedId = id)}
          onenable={() => void intercept.setEnabled(true)}
        />
      </section>

      {#if focus}
        <section class="card detail" data-testid="intercept-detail">
          <Toolbar label="Held request actions">
            <Button
              variant="primary"
              icon="forward"
              kbd="Ctrl+F"
              data-testid="forward"
              disabled={!original}
              onclick={() => void forward()}
              >Forward{dirty && focus.editable ? ' (edited)' : ''}</Button
            >
            <Button variant="danger" icon="drop" kbd="Ctrl+D" data-testid="drop" onclick={() => void drop()}
              >Drop</Button
            >
            <Button
              icon="repeater"
              kbd="Ctrl+R"
              data-testid="to-repeater"
              disabled={!original}
              onclick={toRepeater}>Send to Repeater</Button
            >
            {#if !focus.editable}
              <span class="not-editable" data-testid="not-editable-badge"
                >not editable (body over the hold limit; forwards original)</span
              >
            {/if}
          </Toolbar>
          {#if loadError}
            <p class="alert" role="alert" data-testid="held-load-error">{loadError}</p>
          {:else if original}
            {#key `${focus.id}:${epoch}`}
              <ByteEditor bind:bytes readonly={!focus.editable} />
            {/key}
          {:else}
            <p class="placeholder">Loading held bytes…</p>
          {/if}
        </section>
      {/if}
    </div>

    <aside class="card rules-card">
      <h2>Rules</h2>
      <RulesEditor />
    </aside>
  </div>
</div>

<style>
  .intercept {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
    min-width: 0;
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  .spacer {
    flex: 1 1 auto;
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: var(--s-3);
    cursor: pointer;
  }
  .switch input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
  }
  .track {
    position: relative;
    width: 40px;
    height: 22px;
    border-radius: 11px;
    background: var(--bg-active);
    border: 1px solid var(--border-strong);
    transition: background 0.12s;
  }
  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--fg-muted);
    transition: transform 0.12s;
  }
  .switch input:checked + .track {
    background: var(--warn-bg);
    border-color: var(--warn);
  }
  .switch input:checked + .track .thumb {
    transform: translateX(18px);
    background: var(--warn);
  }
  .switch input:focus-visible + .track {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .state {
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 20rem;
    gap: var(--s-3);
    align-items: start;
  }
  @media (max-width: 900px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .work {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    min-width: 0;
  }
  .card {
    padding: var(--s-3) var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    min-width: 0;
  }
  h2 {
    margin: 0 0 var(--s-2);
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .not-editable {
    font-size: var(--fs-xs);
    color: var(--warn-fg);
    background: var(--warn-bg);
    border-radius: var(--r-1);
    padding: 2px var(--s-2);
  }
  .alert {
    margin: 0;
    padding: var(--s-2) var(--s-3);
    color: var(--danger-fg);
    background: var(--danger-bg);
    border-radius: var(--r-1);
  }
  .placeholder {
    color: var(--fg-muted);
  }
</style>
