// Tiny history-based router: the current path is reactive state derived from
// location.pathname. Views are mapped from the path by the consumer (Task 8
// adds the "/" -> History route).

export function createRouter() {
  let path = $state(location.pathname);
  // The view navigated away from, so a dismissable route (Decoder) can return.
  let previous: string | null = null;
  window.addEventListener('popstate', () => {
    path = location.pathname;
  });
  return {
    get path(): string {
      return path;
    },
    get previous(): string | null {
      return previous;
    },
    navigate(to: string): void {
      if (to === location.pathname) return;
      previous = location.pathname;
      history.pushState(null, '', to);
      path = location.pathname;
    },
  };
}

/**
 * The app's single router instance. Exported so views (Task 8's History) and
 * App.svelte share one reactive `path`; `navigate` is reachable app-wide.
 */
export const router = createRouter();

/** A top-level navigable view. The path->component mapping lives in App.svelte. */
export interface RouteDef {
  path: string;
  label: string;
}

/**
 * The primary-nav route registry, consumed by Shell (to render the nav) and
 * App (to map `router.path` to a view). `/repeater` is Plan 5b's repeater view;
 * `/decoder` and `/settings` are U1 D6.
 */
export const routes: readonly RouteDef[] = [
  { path: '/', label: 'History' },
  { path: '/repeater', label: 'Repeater' },
  { path: '/intercept', label: 'Intercept' },
  { path: '/scope', label: 'Scope' },
  { path: '/decoder', label: 'Decoder' },
  { path: '/settings', label: 'Settings' },
];
