import { expect, test, type Page } from '@playwright/test';

const notebook = {
  notebook_id: 1,
  title: 'Research notes',
  source_count: 1,
  updated_at: 1,
  sources: [],
};

const evidenceA = {
  evidence_id: 42,
  artifact_version: 3,
  source: 'guide.md:4-9',
  range_start: 4,
  range_end: 9,
  score_schema_version: 1,
  scores: [
    {
      score_kind: 'lexical_bm25',
      raw_score: 875,
      raw_rank: { state: 'unavailable', reason: 'not ranked' },
      scale: {
        kind: 'fixed_point',
        name: 'bm25',
        denominator: 1000,
        minimum: 0,
        maximum: 1000,
        higher_is_better: true,
      },
      representation: 'lexical_text_v1',
      fingerprint: 'fingerprint-lexical',
      fingerprint_components: {},
    },
  ],
  trust: 'High',
  freshness: 'Fresh',
};

const evidenceB = {
  evidence_id: 43,
  artifact_version: 3,
  source: 'guide.md:10-15',
  range_start: 10,
  range_end: 15,
  score_schema_version: 1,
  scores: [
    {
      score_kind: 'dense_similarity',
      raw_score: 234,
      raw_rank: { state: 'ranked', rank: 2 },
      scale: {
        kind: 'fixed_point',
        name: 'cosine',
        denominator: 1000,
        minimum: 0,
        maximum: 1000,
        higher_is_better: true,
      },
      representation: 'dense_text_v1',
      fingerprint: 'fingerprint-dense',
      fingerprint_components: {},
    },
  ],
  trust: 'High',
  freshness: 'Fresh',
};

const evidence = {
  evidence_id: 42,
  artifact_id: 10,
  artifact_title: 'Guide',
  artifact_content_hash: 'hash',
  source: { type: 'file', path: 'guide.md', start_line: 4, end_line: 9, content_hash: 'hash' },
  excerpt: 'Axum routes are configured in the router',
  observed_at: 11,
};

const retrieval = {
  index_generation: 1,
  corpus_snapshot: 1,
  fingerprint: 'maestria-core:deterministic-v1',
  lanes: {
    hybrid_state: 'Active',
    hybrid_served_classes: ['DomainTerminology'],
    hybrid_evaluation_id: 'hybrid-dense-2026-08-09',
    hybrid_evaluation_date: '2026-08-09',
    hybrid_report_hash: 'report-hash-hybrid',
    learned_sparse_state: 'Active',
    learned_sparse_model: 'test-sparse-model',
    dense_enabled: true,
    dense_model: 'test-embedding-model',
    repository_code_state: 'Shadow',
    visual_state: 'Shadow',
  },
  promotion_records: {
    learned_sparse: {
      evaluation_id: 'sparse-2026-08-09',
      corpus_id: 'corpus-1',
      evaluation_date: '2026-08-09',
      report_hash: 'report-hash-sparse',
      created_at: '2026-08-09 10:00:00',
    },
    hybrid: {
      evaluation_id: 'hybrid-dense-2026-08-09',
      corpus_id: 'corpus-1',
      evaluation_date: '2026-08-09',
      report_hash: 'report-hash-hybrid',
      created_at: '2026-08-09 10:00:00',
    },
  },
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
          status: { instance_root: 'demo', event_count: 3, task_count: 2 },
          notebooks: { notebooks: [notebook] },
          agents: [{ id: 'omp', label: 'Oh My Pi', status: 'ready', config_options: [] }],
        }),
      });
      return;
    }
    if (path === '/api/search' && request.method() === 'GET') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'search',
          data: {
            query: url.searchParams.get('query'),
            query_id: 7,
            trace_id: 8,
            status: 'Succeeded',
            fingerprint: 'maestria-core:deterministic-v1',
            index_generation: 1,
            evidence: [evidenceA, evidenceB],
            coverage: {
              percent_covered: 100,
              gaps: [],
              distinct_sources: 1,
              distinct_documents: 1,
              distinct_sections: 2,
            },
            conflict_count: 0,
          },
        }),
      });
      return;
    }
    if (path === '/api/evidence/42' && request.method() === 'GET') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ type: 'evidence', data: evidence }),
      });
      return;
    }
    if (path === '/api/retrieval' && request.method() === 'GET') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ type: 'retrieval_status', data: retrieval }),
      });
      return;
    }
    if (path === '/api/tasks' && request.method() === 'GET') {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          type: 'task',
          data: {
            tasks: [
              {
                task_id: 1,
                title: 'Review benchmarks',
                status: 'Succeeded',
                priority: 'Normal',
                evidence_ids: [42, 43],
                validation_report_id: 5,
              },
              {
                task_id: 2,
                title: 'Draft release notes',
                status: 'Running',
                priority: 'Low',
                evidence_ids: [43],
                validation_report_id: null,
              },
            ],
          },
        }),
      });
      return;
    }
    await route.fulfill({ status: 404, contentType: 'application/problem+json', body: JSON.stringify({ type: 'urn:maestria:studio:problem:not-found', title: 'Not found', status: 404, detail: 'The requested resource was not found' }) });
  });
}

test('searches and renders lane provenance', async ({ page }) => {
  await installFixture(page);
  await page.goto('/search#session=test');
  await page.getByLabel('Search query').fill('axum routes');
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByRole('heading', { name: 'guide.md:4-9' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'guide.md:10-15' })).toBeVisible();
  await expect(page.getByText('lexical_bm25: bm25 0.875')).toBeVisible();
  await expect(page.getByText('dense_similarity: cosine 0.234')).toBeVisible();
  await expect(page.getByText('Coverage', { exact: true })).toBeVisible();
  await expect(page.getByText('No gaps')).toBeVisible();
  await expect(page.getByText('1 sources · 1 documents · 2 sections · 0 conflicts')).toBeVisible();
});

test('opens evidence from a search result', async ({ page }) => {
  await installFixture(page);
  await page.goto('/search#session=test');
  await page.getByLabel('Search query').fill('axum routes');
  await page.getByRole('button', { name: 'Search' }).click();
  await page.getByRole('heading', { name: 'guide.md:4-9' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('Axum routes are configured in the router')).toBeVisible();
  await page.getByRole('button', { name: 'Close' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('renders retrieval lanes and promotion records', async ({ page }) => {
  await installFixture(page);
  await page.goto('/retrieval#session=test');
  await expect(page.getByRole('heading', { name: 'Retrieval lanes' })).toBeVisible();
  await expect(page.getByText('DomainTerminology')).toBeVisible();
  await expect(page.getByText('hybrid-dense-2026-08-09').first()).toBeVisible();
  await expect(page.getByText('Active').first()).toBeVisible();
  await expect(page.getByText('sparse-2026-08-09')).toBeVisible();
  await expect(page.getByText('test-sparse-model')).toBeVisible();
  await expect(page.getByText('test-embedding-model')).toBeVisible();
  await expect(page.getByText('Index generation')).toBeVisible();
  await expect(page.getByText('3', { exact: true })).toBeVisible();
});

test('renders tasks with validation status', async ({ page }) => {
  await installFixture(page);
  await page.goto('/tasks#session=test');
  await expect(page.getByRole('heading', { name: 'Review benchmarks' })).toBeVisible();
  await expect(page.getByText('Validated')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Draft release notes' })).toBeVisible();
  await expect(page.getByText('2 evidence')).toBeVisible();
  await expect(page.getByText('1 evidence')).toBeVisible();
});
