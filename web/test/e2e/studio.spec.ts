import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.route('**/api/bootstrap', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({
        status: { instance_root: 'demo', event_count: 0, task_count: 0 },
        notebooks: { notebooks: [] },
        agents: [{ id: 'omp', label: 'Oh My Pi', status: 'agent_unconfigured', config_options: [] }],
      }),
    });
  });
});

test('renders the notebook workspace and safe empty state', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Maestria Studio' })).toBeVisible();
  await expect(page.getByText('Create a notebook to begin')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Ask' })).toBeDisabled();
});

test('keeps create-notebook controls keyboard reachable', async ({ page }) => {
  await page.goto('/');
  const title = page.getByLabel('New notebook');
  await title.fill('Research notes');
  await expect(title).toHaveValue('Research notes');
  await expect(page.getByRole('button', { name: 'Create' })).toBeEnabled();
});
