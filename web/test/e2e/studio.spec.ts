import { expect, test, type Page } from '@playwright/test';

const notebook = {
  notebook_id: 1,
  title: 'Research notes',
  source_count: 1,
  updated_at: 1,
  sources: [],
};

const source = {
  source_key: 'guide.md',
  artifact_id: 10,
  title: 'Guide',
  content_hash: 'hash',
  index_status: 'ready',
  source_kind: 'file',
  available: true,
};

const citation = {
  rank: 1,
  score: 0.93,
  evidence: {
    evidence_id: 42,
    artifact_id: 10,
    artifact_title: 'Guide',
    artifact_content_hash: 'hash',
    source: { type: 'file', path: 'guide.md', start_line: 4, end_line: 9, content_hash: 'hash' },
    excerpt: 'Axum routes',
    observed_at: 11,
  },
};

async function installFixture(page: Page, conflict = false): Promise<void> {
  let draftSaved = false;
  let draftDeleted = false;
  await page.route('**/api/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    if (path === '/api/bootstrap') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          status: { instance_root: 'demo', event_count: 0, task_count: 0 },
          notebooks: { notebooks: [notebook] },
          agents: [{ id: 'omp', label: 'Oh My Pi', status: 'ready', config_options: [] }],
        }),
      });
      return;
    }
    if (path === '/api/notebooks' && request.method() === 'POST') {
      await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify({ data: notebook }) });
      return;
    }
    if (path === '/api/notebooks/1' && request.method() === 'GET') {
      await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ data: notebook }) });
      return;
    }
    if (path === '/api/notebooks/1/sources' && request.method() === 'GET') {
      await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ data: { sources: [source] } }) });
      return;
    }
    if (path === '/api/notebooks/1/sources/guide.md') {
      await route.fulfill({ status: 204, body: '' });
      return;
    }
    if (path === '/api/notebooks/1/drafts' && request.method() === 'GET') {
      const drafts = !draftDeleted && (conflict || draftSaved)
        ? [{ draft_id: 7, title: conflict ? 'Existing' : 'Follow-up', revision: conflict ? 3 : 1 }]
        : [];
      await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ data: { drafts } }) });
      return;
    }
    if (path === '/api/notebooks/1/drafts/7' && request.method() === 'GET') {
      const saved = draftSaved && !conflict;
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          data: {
            draft_id: 7,
            notebook_id: 1,
            title: saved ? 'Follow-up' : 'Existing',
            markdown: saved ? 'A saved draft' : 'Saved text',
            revision: saved ? 1 : 3,
            citations: []
          }
        })
      });
      return;
    }
    if (path === '/api/notebooks/1/drafts' && request.method() === 'POST') {
      draftSaved = true;
      await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ data: { draft_id: 7, revision: 1 } }) });
      return;
    }
    if (path === '/api/notebooks/1/drafts/7' && request.method() === 'PATCH' && conflict) {
      await route.fulfill({
        status: 409,
        contentType: 'application/problem+json',
        body: JSON.stringify({
          type: 'urn:maestria:studio:problem:revision-conflict',
          title: 'Revision conflict',
          status: 409,
          detail: 'The resource changed; reload and retry',
        }),
      });
      return;
    }
    if (path === '/api/notebooks/1/drafts/7' && request.method() === 'DELETE') {
      draftDeleted = true;
      await route.fulfill({ status: 204, body: '' });
      return;
    }
    if (path === '/api/notebooks/1/ask' && request.method() === 'POST') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          answer_markdown: 'Grounded answer',
          citations: [citation],
          context: {
            answerability: 'grounded',
            coverage: { percent_covered: 100, distinct_sources: 1 },
            gaps: [],
            citations: [citation],
            trace_id: 8,
            query_id: 7,
          },
          draft_previews: [{ title: 'Follow-up', markdown: 'A saved draft', citations: [citation] }],
        }),
      });
      return;
    }
    if (path === '/api/notebooks/1/evidence/42' && request.method() === 'GET') {
      await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ data: citation.evidence }) });
      return;
    }
    await route.fulfill({ status: 404, contentType: 'application/problem+json', body: JSON.stringify({ type: 'urn:maestria:studio:problem:not-found', title: 'Not found', status: 404, detail: 'The requested resource was not found' }) });
  });
}

test('renders the safe empty dashboard state', async ({ page }) => {
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
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Workspace' })).toBeVisible();
  await expect(page.getByText('Create a notebook to begin')).toBeVisible();
});

test('creates a notebook and completes the workspace flow', async ({ page }) => {
  await installFixture(page);
  await page.goto('/');
  await page.getByLabel('Notebook title').fill('Research notes');
  await page.getByRole('button', { name: 'Create notebook' }).click();
  await expect(page).toHaveURL(/\/notebooks\/1$/);
  await page.getByRole('link', { name: 'Sources', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Sources' })).toBeVisible();
  await page.getByRole('button', { name: 'Attach' }).click();
  await page.getByRole('link', { name: 'Ask', exact: true }).click();
  await page.getByLabel('Question').fill('How are routes configured?');
  await page.getByRole('button', { name: 'Ask' }).click();
  await expect(page.getByText('Grounded answer')).toBeVisible();
  await expect(page.getByText('Answerability: grounded')).toBeVisible();
  await expect(page.getByText('Coverage: 100% across 1 source(s)')).toBeVisible();
  await page.getByRole('button', { name: /Guide/ }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.getByRole('button', { name: 'Close' }).click();
  await page.getByText('Transfer to Drafts').click();
  await expect(page).toHaveURL(/\/notebooks\/1\/drafts$/);
  await expect(page.locator('#draft-title')).toHaveValue('Follow-up');
  await page.getByRole('button', { name: 'Save draft' }).click();
  await expect(page.getByText('Revision 1')).toBeVisible();
});

test('retains draft text after a revision conflict', async ({ page }) => {
  await installFixture(page, true);
  await page.goto('/notebooks/1/drafts');
  await page.getByRole('button', { name: 'Existing Revision 3', exact: true }).click();
  await page.locator('#draft-title').fill('Unsaved title');
  await page.locator('#draft-markdown').fill('Unsaved markdown');
  await page.getByRole('button', { name: 'Save draft' }).click();
  await expect(page.locator('#draft-title')).toHaveValue('Unsaved title');
  await expect(page.getByText('your editor contents are preserved')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reload saved revision' })).toBeVisible();
});
test('deletes a saved draft and clears its selection', async ({ page }) => {
  await installFixture(page, true);
  await page.goto('/notebooks/1/drafts');
  await page.getByRole('button', { name: 'Existing Revision 3', exact: true }).click();
  await page.getByRole('button', { name: 'Delete Existing' }).click();
  await expect(page.getByText('No saved drafts')).toBeVisible();
  await expect(page.locator('#draft-title')).toHaveValue('');
  await expect(page.getByText('Draft deleted')).toBeVisible();
});
test('continues only to a validated remembered notebook', async ({ page }) => {
  await installFixture(page);
  await page.addInitScript(() => {
    window.sessionStorage.setItem('maestria.studio.notebook', '1');
  });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Continue' })).toBeVisible();
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page).toHaveURL(/\/notebooks\/1$/);
});

test('clears an invalid remembered notebook instead of offering Continue', async ({ page }) => {
  await installFixture(page);
  await page.addInitScript(() => {
    window.sessionStorage.setItem('maestria.studio.notebook', '999');
  });
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Continue' })).toHaveCount(0);
  const remembered = await page.evaluate(() => window.sessionStorage.getItem('maestria.studio.notebook'));
  expect(remembered).toBeNull();
});


test('keeps the dashboard create control keyboard reachable on mobile', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route('**/api/bootstrap', async (route) => {
    await route.fulfill({
      contentType: 'application/json',
      body: JSON.stringify({ status: { instance_root: 'demo', event_count: 0, task_count: 0 }, notebooks: { notebooks: [] }, agents: [] }),
    });
  });
  await page.goto('/');
  const title = page.getByLabel('Notebook title');
  await title.fill('Research notes');
  await expect(page.getByRole('button', { name: 'Create notebook' })).toBeEnabled();
});
