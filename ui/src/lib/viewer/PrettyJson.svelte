<script lang="ts">
  // The Pretty tab for a JSON body: the re-indented text (pretty.ts) in a
  // read-only editor with JSON colouring. The text is derived from the
  // recorded bytes and is display-only; Raw stays the source of truth.
  import ByteEditor from '../editor/ByteEditor.svelte';
  import type { EditorMatches } from './find';

  let {
    text,
    wrap,
    showControls,
    matches = null,
    testid,
  }: {
    text: string;
    wrap: boolean;
    showControls: boolean;
    matches?: EditorMatches | null;
    testid: string;
  } = $props();
</script>

<div class="pretty-json" data-testid={testid}>
  <ByteEditor {text} readonly={true} language="json" {wrap} {showControls} {matches} />
</div>
