import { test, expect } from '@playwright/test';

// Seed e2e gate for the shared harness. Later tasks add their own *.spec.ts
// files here; this one just proves the built bundle boots and mounts.
test('app mounts and shows the root element', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByTestId('app-root')).toBeVisible();
});
