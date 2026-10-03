import type { SearchStatus } from '../model';

type Props = {
  status: SearchStatus;
  query: string;
  resultCount: number;
  actionError: string | null;
  onClear: () => void;
};

export function StatusMessage({ status, query, resultCount, actionError, onClear }: Props) {
  if (actionError) {
    return <div className="status-message error-message" role="alert">{actionError}</div>;
  }
  if (status.kind === 'loading') {
    return <div className="status-message" role="status" aria-live="polite">Loading applications…</div>;
  }
  if (status.kind === 'refreshing') {
    return <div className="status-message" role="status" aria-live="polite">Refreshing applications…</div>;
  }
  if (status.kind === 'error') {
    return <div className="status-message error-message" role="alert">{status.message ?? 'Unable to load applications'}</div>;
  }
  if (status.kind === 'calculation_error') {
    return <div className="status-message error-message" role="alert">{status.message ?? 'That calculation is incomplete or unsupported'}</div>;
  }
  if (query && resultCount === 0) {
    return (
      <div className="status-message empty-state" role="status" aria-live="polite">
        <p>No matching apps or commands</p>
        <button type="button" className="secondary-button" onClick={onClear}>Clear query</button>
      </div>
    );
  }
  if (!query && resultCount === 0) {
    return <div className="status-message empty-state" role="status" aria-live="polite">No applications available</div>;
  }
  if (status.kind === 'ready' && status.message) {
    return <div className="status-message" role="status" aria-live="polite">{status.message}</div>;
  }
  return null;
}
