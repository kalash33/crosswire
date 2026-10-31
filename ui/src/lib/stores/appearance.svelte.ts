// Reactive view of theme.ts so the top-bar toggle and the Settings card stay
// in step. theme.ts owns persistence and the <html data-theme> attribute.

import { theme, type Theme } from '../theme';

function createAppearance() {
  let current = $state<Theme>(theme.get());
  return {
    get theme(): Theme {
      return current;
    },
    set(t: Theme): void {
      theme.set(t);
      current = t;
    },
    toggle(): void {
      current = theme.toggle();
    },
  };
}

export const appearance = createAppearance();
