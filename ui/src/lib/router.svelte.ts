// Tiny history-based router: the current path is reactive state derived from
// location.pathname. Views are mapped from the path by the consumer (Task 8
// adds the "/" -> History route).

export function createRouter() {
  let path = $state(location.pathname);
  window.addEventListener('popstate', () => {
    path = location.pathname;
  });
  return {
    get path(): string {
      return path;
    },
    navigate(to: string): void {
      if (to === location.pathname) return;
      history.pushState(null, '', to);
      path = location.pathname;
    },
  };
}
