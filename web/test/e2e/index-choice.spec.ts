import { expect, test, type Page } from '@playwright/test';

const recommended = {
  path: '/tmp/choice/docs',
  class: 'Recommended',
  policy: { max_file_bytes: 0, skip_generated: false, skip_minified: false },
  file_count: 12,
  total_bytes: 4096,
  children: [],
};

const maybe = {
  path: '/tmp/choice/mixed',
  class: 'Maybe',
  policy: {
    max_file_bytes: 1048576,
    skip_generated: true,
    skip_minified: true,
  },
  file_count: 30,
  total_bytes: 20480,
  children: [],
};

const noise = {
  path: '/tmp/choice/dump',
  class: 'Noise',
  policy: {
    max_file_bytes: 1048576,
    skip_generated: true,
    skip_minified: true,
  },
  file_count: 300,
  total_bytes: 2100,
  children: [],
};

const tree = {
  path: '/tmp/choice',
  class: 'Recommended',
  policy: { max_file_bytes: 0, skip_generated: false, skip_minified: false },
  file_count: 342,
  total_bytes: 26676,
  children: [recommended, maybe, noise],
};

const candidates = {
  root: '/tmp/choice',
  home_root: false,
  tree,
};

async function installFixture(page: Page): Promise<void> {
  await page.route('**/api/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    if (path === '/api/bootstrap') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          status: { instance_root: 'demo', event_count: 0, task_count: 0 },
          notebooks: { notebooks: [] },
          agents: [],
        }),
      });
      return;
    }
    if (path.startsWith('/api/index/candidates/')) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ type: 'index_candidates', data: candidates }),
      });
      return;
    }
    if (path === '/api/index/selection' && request.method() === 'GET') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ type: 'index_selection', data: { profile: null } }),
      });
      return;
    }
    if (path === '/api/index/selection' && request.method() === 'PUT') {
      await route.fulfill({ status: 204, body: '' });
      return;
    }
    if (path === '/api/index/run' && request.method() === 'POST') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ type: 'index_run', data: { submitted: 2, skipped: 1 } }),
      });
      return;
    }
    await route.fulfill({ status: 404, contentType: 'application/json', body: '{}' });
  });
}

test('index choice workspace whitelists and runs a selection', async ({ page }) => {
  await installFixture(page);
  await page.goto('/index#session=test');

  await page.getByLabel('Root path').fill('/tmp/choice');
  await page.getByRole('button', { name: 'Scan' }).click();

  // Recommended directories are checked by default; Maybe and Noise are not.
  await expect(page.getByRole('checkbox', { name: 'Include /tmp/choice/docs' })).toBeVisible();
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/choice/docs' }),
  ).toBeChecked();
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/choice/mixed' }),
  ).not.toBeChecked();
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/choice/dump' }),
  ).not.toBeChecked();

  // Check the Maybe directory, then run the selection.
  await page.getByRole('checkbox', { name: 'Include /tmp/choice/mixed' }).check();

  await page.getByRole('button', { name: 'Index selected' }).click();
  await expect(page.getByText('submitted 2 · skipped 1')).toBeVisible();
});
