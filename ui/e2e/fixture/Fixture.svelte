<script lang="ts">
  import ByteEditor from '../../src/lib/editor/ByteEditor.svelte';
  import HexPane from '../../src/lib/editor/HexPane.svelte';

  // Two mounts behind one entry so the committed production bundle is untouched:
  //   (default)   read-only §7.2 golden  (byte-editor.spec.ts)
  //   #editable   editable  §7.2 golden  (editor-edit.spec.ts)
  // The hash is read once at module load; Playwright navigates to the right URL.
  const editable = typeof location !== 'undefined' && location.hash === '#editable';

  const toHex = (u: Uint8Array) =>
    Array.from(u, (b) => b.toString(16).padStart(2, '0')).join('');

  // ---- Read-only golden: all 256 byte values, 0x00..0xFF. ----
  const input = new Uint8Array(256);
  for (let b = 0; b < 256; b++) input[b] = b;

  // Bound to ByteEditor. After mount ByteEditor assigns this from its ACTUAL
  // CodeMirror document (doc.toString() -> latin1ToBytes), so `buf` is the
  // editor's read-back, not the original array.
  let buf = $state<Uint8Array>(input);
  const inputHex = toHex(input);
  const readbackHex = $derived(toHex(buf));

  // ---- Editable golden: all 256 values + an explicit CRLF pair + a trailing
  // lone CR, so the editable read-back proves \r, \r\n and the C1 range survive.
  const editInput = new Uint8Array([...Array.from({ length: 256 }, (_, b) => b), 0x0d, 0x0a, 0x0d]);
  let editBuf = $state<Uint8Array>(editInput);
  const editInputHex = toHex(editInput);
  const editReadbackHex = $derived(toHex(editBuf));
</script>

{#if editable}
  <!-- Editable mount. Edits flow doc -> bytes; readback proves byte-exactness. -->
  <div data-testid="editable-input-hex" data-hex={editInputHex}></div>
  <div data-testid="editable-readback-hex" data-hex={editReadbackHex}></div>

  <div data-testid="editable-byte-editor">
    <ByteEditor bind:bytes={editBuf} readonly={false} diagnostic={null} />
  </div>
  <HexPane bytes={editBuf} />
{:else}
  <!-- Test probes: input hex (expected) vs the editor's read-back hex (actual). -->
  <div data-testid="input-hex" data-hex={inputHex}></div>
  <div data-testid="readback-hex" data-hex={readbackHex}></div>

  <ByteEditor bind:bytes={buf} readonly={true} diagnostic={null} />
  <HexPane bytes={buf} />
{/if}
