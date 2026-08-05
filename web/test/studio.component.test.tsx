/** @vitest-environment jsdom */
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
    vi.restoreAllMocks();
  });

  it('renders the empty notebook workspace from bootstrap data', async () => {
    render(<QueryClientProvider client={client()}><App /></QueryClientProvider>);
    expect(await screen.findByRole('heading', { name: 'Maestria Studio' })).toBeVisible();
    expect(screen.getByText('Create a notebook to begin')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Ask' })).toBeDisabled();
  });
});
