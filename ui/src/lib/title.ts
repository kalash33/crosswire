// The window title: `project · view · Crosswire`, so several Crosswire windows
// (one per project) are told apart in the task switcher and tab strip. The
// title is set through `document.title`, which is text, never markup.

const APP = 'Crosswire';

/**
 * The window title for the project `name` (null when the server names none)
 * on the view labelled `view` (empty when the path has no view).
 */
export function windowTitle(name: string | null, view: string): string {
  const parts = [name, view].filter((p): p is string => typeof p === 'string' && p.trim() !== '');
  return name !== null && name.trim() !== ''
    ? [...parts, APP].join(' · ')
    : [APP, ...parts].join(' · ');
}
