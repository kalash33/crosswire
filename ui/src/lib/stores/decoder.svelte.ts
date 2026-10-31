// Hand-off from the `decode-selection` chord (or a "Decode selection" button)
// to the Decoder route: the chord seeds, navigates to /decoder, and the route
// consumes the seed. `generation` bumps on every seed so a chord pressed while
// the route is already open reseeds it.

export interface DecoderSeed {
  bytes: Uint8Array;
  nonLatin1: boolean;
}

export function createDecoderStore() {
  let generation = $state(0);
  // Not reactive: read once by the route, keyed on `generation`.
  let pending: DecoderSeed | null = null;

  return {
    get generation(): number {
      return generation;
    },
    seed(bytes: Uint8Array, nonLatin1: boolean): void {
      pending = { bytes, nonLatin1 };
      generation += 1;
    },
    /** The pending seed, once; null when there is none. */
    consume(): DecoderSeed | null {
      const p = pending;
      pending = null;
      return p;
    },
  };
}

export const decoder = createDecoderStore();
