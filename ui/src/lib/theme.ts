// Theme selection (U1 spec D1). The palette lives in theme.css as CSS custom
// properties on :root; this module only flips `data-theme` on <html>. The CSP
// forbids an inline <script> in index.html, so main.ts applies the stored
// theme synchronously before mounting (a one-frame flash is accepted).

export type Theme = 'dark' | 'light';

export const THEME_KEY = 'crosswire.theme';
const DEFAULT: Theme = 'dark';

function isTheme(v: unknown): v is Theme {
  return v === 'dark' || v === 'light';
}

function readStored(): Theme {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return isTheme(v) ? v : DEFAULT;
  } catch {
    // Storage disabled (privacy mode, sandboxed frame): fall back silently.
    return DEFAULT;
  }
}

function persist(t: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, t);
  } catch {
    // Not persisted; the choice still applies for this page load.
  }
}

function apply(t: Theme): void {
  document.documentElement.dataset.theme = t;
}

/** Apply the persisted theme (dark when unset or unreadable). Returns it. */
export function applyStoredTheme(): Theme {
  const t = readStored();
  apply(t);
  return t;
}

export const theme = {
  /** The theme currently applied to <html>, else the stored one. */
  get(): Theme {
    const v = document.documentElement.dataset.theme;
    return isTheme(v) ? v : readStored();
  },
  set(t: Theme): void {
    apply(t);
    persist(t);
  },
  /** Flip dark/light; returns the new theme. */
  toggle(): Theme {
    const next: Theme = theme.get() === 'dark' ? 'light' : 'dark';
    theme.set(next);
    return next;
  },
};
