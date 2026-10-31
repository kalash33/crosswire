<script lang="ts">
  // First-run guidance shown in History while the project has no traffic.
  // Every address is text-interpolated; the only navigation is the "Open CA
  // page" button, which window.open()s an address built from the validated
  // server-reported proxy address (or the documented default), never an href.
  import CopyValue from './CopyValue.svelte';
  import Button from '../ui/Button.svelte';
  import Icon from '../ui/Icon.svelte';
  import { browserCommand, caPageUrl } from './start';

  let {
    proxyAddr,
    apiOrigin,
    projectHint,
    proxyReported = false,
  }: {
    proxyAddr: string;
    apiOrigin: string;
    projectHint?: string;
    /** True when the server reported proxyAddr (else it is the default). */
    proxyReported?: boolean;
  } = $props();

  const caUrl = $derived(caPageUrl(proxyAddr));
  const command = $derived(browserCommand(proxyAddr, projectHint));

  function openCa(): void {
    window.open(caUrl, '_blank', 'noopener');
  }
</script>

<section class="start" data-testid="start-panel" aria-labelledby="start-title">
  <header class="intro">
    <h2 id="start-title">Waiting for the first request</h2>
    <p>Send traffic through the Crosswire proxy and every exchange appears here as it happens.</p>
  </header>

  <div class="steps">
    <div class="step">
      <h3><Icon name="forward" size={14} />Proxy</h3>
      {#if proxyReported}
        <p>The proxy is listening on this address. Set it as the HTTP and HTTPS proxy in your browser or tool.</p>
      {:else}
        <p>
          Use the address <code>crosswire serve</code> printed (default <code>127.0.0.1:8080</code>) as the
          HTTP and HTTPS proxy in your browser or tool.
        </p>
      {/if}
      <CopyValue value={proxyAddr} what="the proxy address" testid="start-proxy" />
    </div>

    <div class="step">
      <h3><Icon name="check" size={14} />Trust the CA</h3>
      <p>
        To read HTTPS, import the Crosswire CA from the proxy's own page: <code>ca.der</code> for the system or
        Chromium store, <code>ca.pem</code> for Firefox and command-line tools.
      </p>
      <CopyValue value={caUrl} what="the CA page address" testid="start-ca" />
      <div class="row">
        <Button size="sm" icon="external" data-testid="start-open-ca" onclick={openCa}>Open CA page</Button>
      </div>
    </div>

    <div class="step">
      <h3><Icon name="play" size={14} />Launch a wired browser</h3>
      <p>Or skip both steps: this starts Chromium in its own profile with the proxy and CA already set.</p>
      <CopyValue value={command} what="the browser command" testid="start-browser-command" wrap />
    </div>
  </div>

  <p class="foot">This window talks to the Crosswire API at <code>{apiOrigin}</code>.</p>
</section>

<style>
  .start {
    margin: var(--s-2) 0;
    padding: var(--s-5) var(--s-5) var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  .intro h2 {
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .intro p {
    margin: var(--s-1) 0 0;
    color: var(--fg-muted);
  }
  .steps {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    margin-top: var(--s-5);
  }
  .step {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    padding: 0 var(--s-5);
    border-left: 1px solid var(--border);
  }
  .step:first-child {
    padding-left: 0;
    border-left: none;
  }
  .step:last-child {
    padding-right: 0;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    margin: 0;
    font-size: var(--fs-md);
    font-weight: 600;
  }
  h3 :global(.icon) {
    color: var(--accent);
  }
  .step p {
    margin: 0;
    max-width: 44ch;
    font-size: var(--fs-sm);
    line-height: 1.5;
    color: var(--fg-muted);
  }
  code {
    font-family: var(--font-mono);
    font-size: 0.95em;
    color: var(--fg);
  }
  .row {
    display: flex;
  }
  .foot {
    margin: var(--s-5) 0 0;
    padding-top: var(--s-3);
    border-top: 1px solid var(--border-faint);
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  @media (max-width: 1100px) {
    .steps {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--s-4);
    }
    .step,
    .step:first-child {
      padding: var(--s-4) 0 0;
      border-left: none;
      border-top: 1px solid var(--border-faint);
    }
    .step:first-child {
      padding-top: 0;
      border-top: none;
    }
  }
</style>
