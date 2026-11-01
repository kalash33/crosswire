<script lang="ts">
  // The /decoder route (U1 D6): hosts the chained DecoderPane full-height. The
  // decode-selection chord seeds it through the decoder store and navigates
  // here; a chord pressed while already here reseeds (the store generation).
  import DecoderPane from '../lib/components/DecoderPane.svelte';
  import { decoder, type DecoderSeed } from '../lib/stores/decoder.svelte';

  const EMPTY: DecoderSeed = { bytes: new Uint8Array(), nonLatin1: false };

  let seed = $state<DecoderSeed>(decoder.consume() ?? EMPTY);

  $effect(() => {
    void decoder.generation; // re-run on every seed
    const next = decoder.consume();
    if (next) seed = next;
  });

</script>

<div class="decoder-view" data-testid="decoder-view">
  {#key seed}
    <DecoderPane initial={seed.bytes} initialNonLatin1={seed.nonLatin1} />
  {/key}
</div>

<style>
  .decoder-view {
    min-height: 100%;
  }
</style>
