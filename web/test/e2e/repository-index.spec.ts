import { expect, test, type Page } from '@playwright/test';

const recommended = {
  path: '/tmp/repo/crates',
  class: 'Recommended',
  policy: { max_file_bytes: 0, skip_generated: false, skip_minified: false },
  file_count: 40,
  total_bytes: 20480,
  children: [],
};

const maybe = {
  path: '/tmp/repo/legacy',
  class: 'Maybe',
  policy: {
    max_file_bytes: 1048576,
    skip_generated: true,
    skip_minified: true,
  },
  file_count: 15,
  total_bytes: 10240,
  children: [],
};

const noise = {
  path: '/tmp/repo/dump',
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
  path: '/tmp/repo',
  class: 'Recommended',
  policy: { max_file_bytes: 0, skip_generated: false, skip_minified: false },
  file_count: 356,
  total_bytes: 32820,
  children: [recommended, maybe, noise],
};

const candidates = {
  root: '/tmp/repo',
  tree,
};

const summary = {
  repository_root: '/tmp/repo',
  commit_sha: 'abc123',
  worktree_identity: 'wt-1',
  parser_generation: 'repository-code-v4',
  package_count: 1,
  symbol_count: 12,
  file_count: 2,
  selected_paths: ['crates'],
  changed_files: 0,
  changed_symbols: 0,
  workspace_warnings: [],
};

// Captured repository-index run payloads for assertion.
const runBodies: string[] = [];

async function installFixture(page: Page): Promise<void> {
  await page.route('**/api/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    if (path === '/api/repository-index/children' && request.method() === 'POST') {
      const body = request.postDataJSON();
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'repository_index_children',
          data: {
            root: body.root,
            path: body.path,
            children:
              body.path === 'crates'
                ? [
                    {
                      path: '/tmp/repo/crates/one',
                      class: 'Recommended',
                      policy: {
                        max_file_bytes: 0,
                        skip_generated: false,
                        skip_minified: false,
                      },
                      file_count: 2,
                      total_bytes: 1024,
                      children: [],
                    },
                  ]
                : [],
          },
        }),
      });
      return;
    }
    if (path === '/api/repository-index/progress' && request.method() === 'GET') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'repository_index_progress',
          data: { progress: { phase: 'registering', total: 813, registered: 42 } },
        }),
      });
      return;
    }
    if (path === '/api/repository-index/files' && request.method() === 'POST') {
      const body = request.postDataJSON();
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'repository_index_files',
          data: {
            root: body.root,
            path: body.path,
            files:
              body.path === 'legacy'
                ? [
                    {
                      path: 'legacy/old.rs',
                      size: 2048,
                      kind: 'code',
                    },
                  ]
                : body.path === 'crates'
                  ? [
                      {
                        path: 'crates/one/lib.rs',
                        size: 512,
                        kind: 'code',
                      },
                    ]
                  : [],
            truncated: false,
          },
        }),
      });
      return;
    }
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
    if (path.startsWith('/api/repository-index/candidates/')) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ type: 'repository_index_candidates', data: candidates }),
      });
      return;
    }
    if (path === '/api/repository-index/run' && request.method() === 'POST') {
      runBodies.push(request.postData() ?? '');
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'repository_index_run',
          data: { mode: 'full', summary, registered: 2, skipped: 0 },
        }),
      });
      return;
    }
    if (path.startsWith('/api/repository-index/status/')) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'repository_index_status',
          data: {
            root: '/tmp/repo',
            present: true,
            summary,
            freshness: {
              status: 'current',
              data: {
                indexed: { commit_sha: 'abc123', worktree_identity: 'wt-1' },
                current: { commit_sha: 'abc123', worktree_identity: 'wt-1' },
              },
            },
            progress: { phase: 'registering', total: 813, registered: 42 },
          },
        }),
      });
      return;
    }
    await route.fulfill({ status: 404, contentType: 'application/json', body: '{}' });
  });
}

test('repository index workspace selects, runs, and reports status', async ({ page }) => {
  await installFixture(page);
  await page.goto('/index#session=test');

  // The repository code index is the Repositories subsection of Index.
  await page.getByRole('link', { name: 'Repositories' }).click();
  await expect(page).toHaveURL(/\/index\/repositories/);

  await page.getByLabel('Repository root').fill('/tmp/repo');
  await page.getByRole('button', { name: 'Scan' }).click();

  // Recommended directories are checked by default; Maybe and Noise are not.
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/repo/crates' }),
  ).toBeVisible();
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/repo/crates' }),
  ).toBeChecked();
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/repo/legacy' }),
  ).not.toBeChecked();
  await expect(
    page.getByRole('checkbox', { name: 'Include /tmp/repo/dump' }),
  ).not.toBeChecked();

  // The scan loads the persisted status panel: present, current, selection.
  await expect(page.getByText('present · current')).toBeVisible();
  await expect(page.getByText('selected: crates')).toBeVisible();

  // The status panel carries the live run progress indicator.
  await expect(page.getByText('registering 42/813 sources')).toBeVisible();

  // Drill deeper: expand the Maybe directory and select one of its files.
  await page.getByRole('button', { name: 'Expand /tmp/repo/legacy' }).click();
  await expect(page.getByRole('checkbox', { name: 'Include legacy/old.rs' })).toBeVisible();
  await page.getByRole('checkbox', { name: 'Include legacy/old.rs' }).check();

  // Files inside a selected Recommended directory are included via their
  // parent and cannot be unchecked at the file level.
  await page.getByRole('button', { name: 'Expand /tmp/repo/crates' }).click();
  await expect(page.getByRole('checkbox', { name: 'Include crates/one/lib.rs' })).toBeVisible();
  await expect(
    page.getByRole('checkbox', { name: 'Include crates/one/lib.rs' }),
  ).toBeChecked();
  await expect(
    page.getByRole('checkbox', { name: 'Include crates/one/lib.rs' }),
  ).toBeDisabled();

  // Check the Maybe directory, then run the selection: the run posts the
  // selected includes (directories and the individually selected file) and
  // the result panel shows mode and counts.
  await page.getByRole('checkbox', { name: 'Include /tmp/repo/legacy' }).check();

  await page.getByRole('button', { name: 'Index selected' }).click();
  await expect(
    page.getByText('mode=full · 12 symbols · 2 files · registered 2 · skipped 0'),
  ).toBeVisible();
  await expect(page.getByText('present · current')).toBeVisible();

  // The run payload carries both the directory and the file selection.
  const lastRun = JSON.parse(runBodies.at(-1) ?? '{}');
  const includes = lastRun.includes ?? [];
  expect(includes).toContain('/tmp/repo/legacy');
  expect(includes).toContain('legacy/old.rs');
  expect(includes).toContain('/tmp/repo/crates');
});
