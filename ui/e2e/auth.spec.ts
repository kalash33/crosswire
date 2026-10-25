import { test, expect, type Page } from '@playwright/test';

const GOOD = 'good-token';
const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0,
};

// No backend: emulate the session endpoint and the cookie it would set.
async function mockApi(page: Page) {
  let authed = false;
  await page.route('**/api/v1/session', async (route) => {
    const body = route.request().postDataJSON() as { bootstrap?: string };
    if (body.bootstrap === GOOD) {
      authed = true;
      await route.fulfill({ status: 200, json: { expires_in: 1800 } });
    } else {
      await route.fulfill({
        status: 401,
        json: { error: { code: 'unauthorized', message: 'invalid bootstrap token' } },
      });
    }
  });
  await page.route('**/api/v1/counts', async (route) => {
    if (authed) await route.fulfill({ status: 200, json: COUNTS });
    else
      await route.fulfill({
        status: 401,
        json: { error: { code: 'unauthorized', message: 'no session' } },
      });
  });
}

test.beforeEach(async ({ page }) => mockApi(page));

test('bootstrap fragment enters the app and the token leaves the URL', async ({ page }) => {
  await page.goto(`/#bootstrap=${GOOD}`);
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await expect(page.getByTestId('auth-screen')).toHaveCount(0);
  expect(page.url()).not.toContain(GOOD);
  expect(page.url()).not.toContain('bootstrap');
});

test('no token shows the auth screen', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByTestId('auth-screen')).toBeVisible();
  await expect(page.getByTestId('app-shell')).toHaveCount(0);
});

test('an empty bootstrap fragment lands on the auth screen with the URL cleared', async ({ page }) => {
  await page.goto('/#bootstrap=');
  await expect(page.getByTestId('auth-screen')).toBeVisible();
  expect(page.url()).not.toContain('bootstrap');
});

test('pasting a valid token enters the app; invalid shows an error', async ({ page }) => {
  await page.goto('/');
  await page.getByTestId('token-input').fill('wrong');
  await page.getByTestId('token-submit').click();
  await expect(page.getByTestId('auth-error')).toContainText('invalid bootstrap token');
  await page.getByTestId('token-input').fill(`http://localhost:8080/#bootstrap=${GOOD}`);
  await page.getByTestId('token-submit').click();
  await expect(page.getByTestId('app-shell')).toBeVisible();
});
