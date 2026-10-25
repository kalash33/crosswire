<script lang="ts">
  import ByteEditor from '../../src/lib/editor/ByteEditor.svelte';
  import HexPane from '../../src/lib/editor/HexPane.svelte';

  // The §7.2 golden buffer: all 256 byte values, 0x00..0xFF.
  const input = new Uint8Array(256);
  for (let b = 0; b < 256; b++) input[b] = b;

  // Bound to ByteEditor. After mount ByteEditor assigns this from its ACTUAL
  // CodeMirror document (doc.toString() -> latin1ToBytes), so `buf` is the
  // editor's read-back, not the original array.
  let buf = $state<Uint8Array>(input);

  const toHex = (u: Uint8Array) =>
    Array.from(u, (b) => b.toString(16).padStart(2, '0')).join('');

  const inputHex = toHex(input);
  const readbackHex = $derived(toHex(buf));
</script>

<!-- Test probes: input hex (expected) vs the editor's read-back hex (actual). -->
<div data-testid="input-hex" data-hex={inputHex}></div>
<div data-testid="readback-hex" data-hex={readbackHex}></div>

<ByteEditor bind:bytes={buf} readonly={true} diagnostic={null} />
<HexPane bytes={buf} />
