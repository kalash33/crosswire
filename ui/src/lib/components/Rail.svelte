<script lang="ts">
  // The tool rail: one icon+label button per primary view. SPA nav goes
  // through router.navigate (pushState), rendered as buttons so no dynamic
  // <a href> is introduced (§11.1). Settings sits apart at the bottom.
  import { router, routes } from '../router.svelte';
  import Icon from '../ui/Icon.svelte';
  import type { IconName } from '../ui/icons';

  const ICON_FOR: Record<string, IconName> = {
    '/': 'history',
    '/repeater': 'repeater',
    '/intercept': 'intercept',
    '/scope': 'scope',
    '/decoder': 'decoder',
    '/settings': 'settings',
  };
  const tools = routes.filter((r) => r.path !== '/settings');
  const tail = routes.filter((r) => r.path === '/settings');
</script>

{#snippet item(route: { path: string; label: string })}
  <button
    type="button"
    class="nav-link"
    class:active={router.path === route.path}
    aria-current={router.path === route.path ? 'page' : undefined}
    data-testid={`nav-${route.label.toLowerCase()}`}
    title={`${route.label} (${route.path})`}
    onclick={() => router.navigate(route.path)}
  >
    <Icon name={ICON_FOR[route.path] ?? 'chevron-right'} size={18} />
    <span class="nav-label">{route.label}</span>
  </button>
{/snippet}

<nav class="rail" data-testid="app-nav" aria-label="Primary">
  {#each tools as route (route.path)}{@render item(route)}{/each}
  <div class="tail">
    {#each tail as route (route.path)}{@render item(route)}{/each}
  </div>
</nav>

<style>
  .rail {
    grid-area: rail;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding: var(--s-1);
    min-height: 0;
    overflow-y: auto;
    background: var(--bg-elev);
    border-right: 1px solid var(--border);
  }
  .tail {
    margin-top: auto;
    padding-top: var(--s-1);
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
  }
  .nav-link {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    flex: none;
    height: 46px;
    min-height: 0;
    padding: 0;
    color: var(--fg-muted);
    background: transparent;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
    position: relative;
  }
  .nav-link:hover:not(.active) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .nav-link.active {
    color: var(--fg);
    background: var(--accent-soft);
  }
  .nav-link.active::before {
    content: '';
    position: absolute;
    left: calc(-1 * var(--s-1));
    top: 10px;
    bottom: 10px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--accent);
  }
  .nav-link.active :global(.icon) {
    color: var(--accent);
  }
  .nav-label {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 10px;
    line-height: 1.1;
    letter-spacing: 0.01em;
  }
</style>
