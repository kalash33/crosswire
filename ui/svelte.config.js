import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

// Plain client-rendered SPA: Svelte 5 with runes, no SvelteKit, no SSR.
export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    runes: true,
  },
};
