import { test, expect } from '@playwright/test';

// The §7.2 EDITABLE golden. An editable ByteEditor is seeded with every byte
// value 0x00..0xFF plus an explicit CRLF pair and a trailing lone CR. We edit
// ONE byte through the real CodeMirror document (type a char at offset 0) and
// read the WHOLE buffer back out of the editor — it must equal the seed with
// that single edit applied, byte for byte: no line-ending normalization (lone
// \r and \r\n survive), no C1-range (0x80..0x9F) corruption, no placeholder
// leaking into the real bytes.

const toHex = (bytes: number[]) => bytes.map((b) => b.toString(16).padStart(2, '0')).join('');

// Must mirror e2e/fixture/Fixture.svelte's editable seed exactly.
const SEED: number[] = [
  ...Array.from({ length: 256 }, (_, b) => b), // 0x00..0xFF (incl. C1 0x80..0x9F, lone \r at 0x0d)
  0x0d,
  0x0a, // an explicit CRLF pair
  0x0d, // a trailing lone CR
];

const SEED_HEX = toHex(SEED);
// Typing 'Z' (0x5A) at document offset 0 prepends one byte; the rest is intact.
const EDITED_HEX = toHex([0x5a, ...SEED]);

test('editable byte editor: edit one byte, read the full buffer back byte-exact', async ({
  page,
}) => {
  await page.goto('/e2e/fixture/index.html#editable');

  const editor = page.getByTestId('editable-byte-editor');
  await expect(editor).toBeVisible();

  // The seed round-trips through the EDITABLE document before any edit.
  await expect(page.getByTestId('editable-input-hex')).toHaveAttribute('data-hex', SEED_HEX);
  await expect(page.getByTestId('editable-readback-hex')).toHaveAttribute('data-hex', SEED_HEX);

  // Edit one byte through the real editor: focus places the caret at offset 0
  // (CodeMirror's initial selection), then type one printable character.
  const content = editor.locator('.cm-content');
  await content.focus();
  await page.keyboard.type('Z');

  // The whole buffer reads back byte-exact with just that one byte prepended —
  // lone \r, \r\n and the C1 range all preserved.
  await expect(page.getByTestId('editable-readback-hex')).toHaveAttribute('data-hex', EDITED_HEX);

  // The hex pane (same bytes) agrees. Only one editor renders per page in
  // editable mode, so the plain hex-pane testid is unambiguous here.
  await expect(page.getByTestId('hex-pane')).toHaveAttribute('data-hex', EDITED_HEX);

  // Non-printables are still drawn as view-only placeholders over the live doc.
  await expect(editor.locator('.cm-nonprintable').first()).toBeVisible();
});
