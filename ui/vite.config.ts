/// <reference types="vitest/config" />
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Dev-only proxy target. The production app is served same-origin by
// crosswire-server under a strict CSP; this proxy exists ONLY for `vite dev`,
// where the SPA runs on its own port and must reach a separately-running
// `crosswire serve`. Override with CROSSWIRE_DEV_SERVER / CROSSWIRE_DEV_WS
// (e.g. CROSSWIRE_DEV_SERVER=http://127.0.0.1:9090 npm run dev).
const DEV_SERVER = process.env.CROSSWIRE_DEV_SERVER ?? 'http://127.0.0.1:8080';
const DEV_WS = process.env.CROSSWIRE_DEV_WS ?? 'ws://127.0.0.1:8080';

export default defineConfig({
  plugins: [svelte()],

  server: {
    proxy: {
      // The event stream is a WebSocket; it must be matched BEFORE the generic
      // /api rule and forwarded with ws:true.
      '/api/v1/events': {
        target: DEV_WS,
        ws: true,
        changeOrigin: true,
      },
      // All other API traffic (plain HTTP/JSON).
      '/api': {
        target: DEV_SERVER,
        changeOrigin: true,
      },
    },
  },

  build: {
    // ---------------------------------------------------------------------
    // Reproducible build (hard requirement): the bundle is committed into the
    // server crate and a CI drift-check rebuilds and byte-diffs it, so output
    // must be byte-stable across runs on identical inputs.
    //
    //  * Asset file names use Rollup's [hash], which is a CONTENT hash derived
    //    purely from bundle contents — deterministic for identical inputs, and
    //    never time- or environment-derived.
    //  * sourcemap:false — source maps can embed absolute paths / machine state.
    //  * No plugin here injects a build timestamp or banner; Vite/Rollup add
    //    none by default.
    //  * reportCompressedSize:false — cosmetic only; keeps builds hermetic and
    //    avoids gzip-size work that has no effect on output bytes.
    // ---------------------------------------------------------------------
    sourcemap: false,
    reportCompressedSize: false,
    rollupOptions: {
      output: {
        entryFileNames: 'assets/[name]-[hash].js',
        chunkFileNames: 'assets/[name]-[hash].js',
        assetFileNames: 'assets/[name]-[hash][extname]',
      },
    },
  },

  // Component tests run in jsdom. Svelte 5's client runtime is selected via the
  // `browser` export condition, applied ONLY under Vitest so the production
  // build's resolution is untouched. (vite-plugin-svelte FAQ pattern.)
  resolve: {
    conditions: process.env.VITEST ? ['browser'] : undefined,
  },

  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    globals: false,
  },
});
