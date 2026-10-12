import {chromium, expect} from '@playwright/test';
const [origin, brief] = process.argv.slice(2);
if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(origin) || !/^[0-9a-f-]{36}$/.test(brief)) throw Error('Owned loopback fixture only');
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.goto(origin);
  await page.getByRole('link', {name: 'Sign in with your organization'}).click();
  await expect(page.getByRole('button', {name: 'Sign out', exact: true})).toBeVisible();
  await page.goto(`${origin}/?brief=${brief}`);
  await expect(page.getByRole('heading', {name: 'Before changing this code'})).toBeVisible();
  await expect(page.getByText(/Bob Synthetic/).first()).toBeVisible();
  await expect(page.getByText(/billing worker deduplicates by that id/).first()).toBeVisible();
  const acknowledge = page.getByRole('button', {name: 'Acknowledge this change'});
  await expect(acknowledge).toBeDisabled();
  await page.getByLabel('I reviewed the exact targets, operation, sources, and gaps shown above.').check();
  await acknowledge.click();
  await expect(page.getByText('Acknowledged for this operation only.')).toBeVisible();
  await expect(acknowledge).toBeDisabled();
  console.log(JSON.stringify({real_browser_acknowledgement: true, confirmed_contributor_and_rationale_visible: true}));
} finally { await browser.close(); }
