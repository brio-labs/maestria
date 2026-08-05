/** @vitest-environment jsdom */
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { App } from '../src/app';

function client(): QueryClient {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

describe('Studio App', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ notebooks: { notebooks: [] }, agents: [] }), {
      headers: { 'Content-Type': 'application/json' },
    })));
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders the empty notebook workspace from bootstrap data', async () => {
    render(<QueryClientProvider client={client()}><App /></QueryClientProvider>);
    expect(await screen.findByRole('heading', { name: 'Maestria Studio' })).toBeVisible();
    expect(screen.getByText('Create a notebook to begin')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Ask' })).toBeDisabled();
  });

  it('creates a notebook after the async request completes', async () => {
    const notebook = { notebook_id: 1, title: 'Research notes', source_count: 0, sources: [] };
    vi.stubGlobal('fetch', vi.fn(async (request: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(request), 'http://studio.local').pathname;
      if (path === '/api/bootstrap') {
        return new Response(JSON.stringify({ notebooks: { notebooks: [] }, agents: [{ id: 'omp', label: 'Oh My Pi', status: 'ready' }] }));
      }
      if (path === '/api/notebooks' && init?.method === 'POST') {
        return new Response(JSON.stringify({ type: 'notebook', data: notebook }), { status: 201 });
      }
      if (path === '/api/notebooks') {
        return new Response(JSON.stringify({ type: 'notebook_list', data: { notebooks: [notebook] } }));
      }
      if (path === '/api/notebooks/1') {
        return new Response(JSON.stringify({ type: 'notebook', data: notebook }));
      }
      if (path === '/api/notebooks/1/sources') {
        return new Response(JSON.stringify({ type: 'notebook_sources', data: { sources: [] } }));
      }
      if (path === '/api/notebooks/1/drafts') {
        return new Response(JSON.stringify({ type: 'notebook_drafts', data: { drafts: [] } }));
      }
      throw new Error(`unexpected request: ${path}`);
    }));

    render(<QueryClientProvider client={client()}><App /></QueryClientProvider>);
    const input = await screen.findByPlaceholderText('Notebook title');
    fireEvent.change(input, { target: { value: 'Research notes' } });
    fireEvent.submit(document.getElementById('create')!);

    expect(await screen.findByRole('heading', { name: 'Research notes' })).toBeVisible();
    expect(input).toHaveValue('');
  });
});
