import { toasts } from './ui/toasts.svelte';

/** Copy `text` and report the outcome as a toast ("Copied <what>."). */
export async function copyWithToast(text: string, what: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    toasts.push({ kind: 'success', text: `Copied ${what}.` });
    return true;
  } catch {
    toasts.push({ kind: 'error', text: `Could not copy ${what}: the clipboard is unavailable.` });
    return false;
  }
}
