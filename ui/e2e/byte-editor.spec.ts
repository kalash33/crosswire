import { test, expect } from '@playwright/test';

// The §7.2 golden, read-only direction: a captured buffer containing ALL 256
// byte values renders in the editor, the hex pane matches it, and reading the
// editor's bytes back equals the input exactly.
const EXPECTED_HEX = Array.from({ length: 256 }, (_, b) =>
  b.toString(16).padStart(2, '0'),
).join('');

test('all 256 bytes: editor read-back and hex pane equal the input', async ({ page }) => {
  await page.goto('/e2e/fixture/index.html');

  await expect(page.getByTestId('byte-editor')).toBeVisible();

  // Sanity: the expected buffer really is the full 0x00..0xFF range.
  await expect(page.getByTestId('input-hex')).toHaveAttribute('data-hex', EXPECTED_HEX);

  // Read the editor's bytes back out of its real CodeMirror document — must
  // equal the input byte for byte (no line-ending normalization, no lossy ops).
  await expect(page.getByTestId('readback-hex')).toHaveAttribute('data-hex', EXPECTED_HEX);

  // The hex pane shows exactly the same bytes.
  await expect(page.getByTestId('hex-pane')).toHaveAttribute('data-hex', EXPECTED_HEX);

  // Non-printables are drawn as view-only placeholder widgets while the document
  // still holds the real bytes (proven by the read-back above).
  await expect(page.locator('.cm-nonprintable').first()).toBeVisible();
  const placeholderCount = await page.locator('.cm-nonprintable').count();
  expect(placeholderCount).toBeGreaterThan(0);
});
