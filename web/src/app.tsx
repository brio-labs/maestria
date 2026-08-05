import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, ReactElement } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import ReactMarkdown from 'react-markdown';
type Json = Record<string, unknown>;

type ApiError = Error & { code?: string; status?: number };

interface NotebookSummary {
  notebook_id: number;
  title: string;
  source_count?: number;
  updated_at?: number;
}
interface SourceSelection { source_key: string; available: boolean; artifact_id?: number | null }
interface Notebook extends NotebookSummary { sources: SourceSelection[] }
interface CatalogSource {
  source_key: string;
  artifact_id?: number | null;
  title?: string | null;
  content_hash?: string | null;
  index_status: string;
  source_kind?: string;
  available: boolean;
}
interface DraftSummary { draft_id: number; title: string; revision: number }
interface FrozenCitation {
  evidence_id: number;
  artifact_id: number;
  artifact_title: string;
  artifact_content_hash: string;
  source: string;
  excerpt: string;
  observed_at: number;
}
interface Evidence {
  evidence_id: number;
  artifact_id: number;
  artifact_title: string;
  artifact_content_hash?: string;
  excerpt: string;
  observed_at: number;
  source?: Json;
}
interface Citation { rank: number; score: number; evidence: Evidence }
interface Context {
  answerability?: string;
  coverage?: { percent_covered?: number; distinct_sources?: number };
  gaps?: string[];
  citations?: Citation[];
  trace_id?: number;
  query_id?: number;
  source_selection_digest?: string;
}
interface Agent { id: string; label: string; status: string; config_options?: string[] }
interface Bootstrap { notebooks: { notebooks?: NotebookSummary[] } | NotebookSummary[]; agents?: Agent[] }
interface Draft {
  draft_id: number;
  notebook_id: number;
  title: string;
  markdown: string;
  revision: number;
  citations: FrozenCitation[];
}
interface Preview {
  draftId: number | null;
  revision: number | null;
  title: string;
  markdown: string;
  evidenceIds: number[];
  savedTitle?: string;
  savedMarkdown?: string;
}

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
function isJsonRecord(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function getBearer(): string {
  const hash = new URLSearchParams(window.location.hash.slice(1));
  const session = hash.get('session');
  if (session) {
    sessionStorage.setItem('maestria.studio.bearer', session);
    history.replaceState(null, '', window.location.pathname + window.location.search);
  }
  return sessionStorage.getItem('maestria.studio.bearer') ?? '';
}

async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      Authorization: `Bearer ${getBearer()}`,
      ...(init.body ? { 'Content-Type': 'application/json' } : {}),
      ...(init.headers ?? {}),
    },
  });
  let payload: unknown = {};
  try { payload = await response.json(); } catch { /* empty error body */ }
  if (!response.ok) {
    const payloadRecord = isJsonRecord(payload) ? payload : undefined;
    const detail = isJsonRecord(payloadRecord?.error) ? payloadRecord.error : undefined;
    const error = new Error(String(detail?.message ?? `Studio request failed (${response.status})`)) as ApiError;
    error.code = typeof detail?.code === 'string' ? detail.code : undefined;
    error.status = response.status;
    throw error;
  }
  return payload as T;
}

function unwrap<T>(value: T | { data?: T }): T {
  if (typeof value === 'object' && value !== null && 'data' in value && value.data !== undefined) {
    return value.data as T;
  }
  return value as T;
}

function sourceDescription(source?: Json): string {
  if (!source) return 'Unknown source';
  const kind = String(source.type ?? source.source_kind ?? 'unknown');
  if (kind === 'file') return `${String(source.path ?? 'file')}:${String(source.start_line ?? '?')}-${String(source.end_line ?? '?')}`;
  if (kind === 'web') return String(source.url ?? 'web source');
  if (kind === 'pdf' || kind === 'pdf_region') return `PDF snapshot ${String(source.snapshot_id ?? '?')}`;
  return `${kind} source`;
}

export function App(): ReactElement {
  const { data: bootstrap, isLoading: booting, error: bootstrapError } = useQuery({
    queryKey: ['studio-bootstrap'],
    queryFn: () => api<Bootstrap>('/api/bootstrap'),
  });
  const [notebooks, setNotebooks] = useState<NotebookSummary[]>([]);
  const [notebookId, setNotebookId] = useState<number | null>(null);
  const [notebook, setNotebook] = useState<Notebook | null>(null);
  const [catalog, setCatalog] = useState<CatalogSource[]>([]);
  const [drafts, setDrafts] = useState<DraftSummary[]>([]);
  const [answer, setAnswer] = useState('Ask a question to see a source-grounded answer.');
  const [answerState, setAnswerState] = useState('');
  const [context, setContext] = useState<Context | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [evidence, setEvidence] = useState<Evidence | null>(null);
  const [error, setError] = useState<ApiError | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState('');
  const [history, setHistory] = useState<{ role: 'user' | 'assistant'; markdown: string }[]>([]);
  const generation = useRef(0);
  const selectedSourceKeys = useMemo(() => new Set((notebook?.sources ?? []).filter((source) => source.available).map((source) => source.source_key)), [notebook]);
  const agents = bootstrap?.agents ?? [];
  const agent = agents[0];

  const showFailure = useCallback((reason: unknown) => {
    const next = reason instanceof Error ? reason as ApiError : new Error(String(reason));
    setError(next);
    setStatus('Action failed');
  }, []);

  const resetAnswer = useCallback(() => {
    setContext(null);
    setAnswer('Ask a question to see a source-grounded answer.');
    setAnswerState('');
    setEvidence(null);
  }, []);

  const readSnapshot = useCallback(async (id: number) => {
    const [notebookResponse, sourceResponse, draftResponse, listResponse] = await Promise.all([
      api<Notebook>(`/api/notebooks/${id}`),
      api<{ sources?: CatalogSource[] }>(`/api/notebooks/${id}/sources`),
      api<{ drafts?: DraftSummary[] }>(`/api/notebooks/${id}/drafts`),
      api<{ notebooks?: NotebookSummary[] }>('/api/notebooks'),
    ]);
    return {
      notebook: unwrap(notebookResponse),
      catalog: unwrap(sourceResponse).sources ?? [],
      drafts: unwrap(draftResponse).drafts ?? [],
      notebooks: unwrap(listResponse).notebooks ?? [],
    };
  }, []);

  const selectNotebook = useCallback(async (id: number) => {
    const token = ++generation.current;
    setBusy(true);
    setError(null);
    setStatus('Loading notebook…');
    try {
      const snapshot = await readSnapshot(id);
      if (token !== generation.current) return;
      setNotebookId(id);
      setNotebook(snapshot.notebook);
      setCatalog(snapshot.catalog);
      setDrafts(snapshot.drafts);
      setNotebooks(snapshot.notebooks);
      sessionStorage.setItem('maestria.studio.notebook', String(id));
      setPreview(null);
      setHistory([]);
      resetAnswer();
      setStatus('Notebook ready');
    } catch (reason) {
      if (token === generation.current) showFailure(reason);
    } finally {
      if (token === generation.current) setBusy(false);
    }
  }, [readSnapshot, resetAnswer, showFailure]);

  useEffect(() => {
    if (!bootstrap) return;
    const entries = Array.isArray(bootstrap.notebooks) ? bootstrap.notebooks : bootstrap.notebooks.notebooks ?? [];
    setNotebooks(entries);
    if (notebookId === null && entries.length > 0) {
      const remembered = Number(sessionStorage.getItem('maestria.studio.notebook'));
      const selected = entries.find((entry) => entry.notebook_id === remembered) ?? entries[0];
      void selectNotebook(selected.notebook_id);
    } else if (entries.length === 0) {
      setStatus('Create a notebook to begin');
    }
  }, [bootstrap, notebookId, selectNotebook]);

  useEffect(() => {
    if (bootstrapError) showFailure(bootstrapError);
    else if (booting && !bootstrap) setStatus('Loading Studio…');
  }, [bootstrapError, booting, showFailure, bootstrap]);

  const refresh = useCallback(async () => {
    if (notebookId === null) return false;
    const token = generation.current;
    try {
      const snapshot = await readSnapshot(notebookId);
      if (token !== generation.current) return false;
      setNotebook(snapshot.notebook);
      setCatalog(snapshot.catalog);
      setDrafts(snapshot.drafts);
      setNotebooks(snapshot.notebooks);
      return true;
    } catch (reason) {
      if (token === generation.current) showFailure(reason);
      return false;
    }
  }, [notebookId, readSnapshot, showFailure]);

  const createNotebook = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const title = new FormData(event.currentTarget).get('title')?.toString().trim();
    if (!title) return;
    try {
      setError(null); setStatus('Creating notebook…');
      const created = unwrap(await api<NotebookSummary>('/api/notebooks', { method: 'POST', body: JSON.stringify({ title }) }));
      event.currentTarget.reset();
      await selectNotebook(created.notebook_id);
      setStatus('Notebook created');
    } catch (reason) { showFailure(reason); }
  };

  const renameNotebook = async () => {
    if (!notebook || notebookId === null) return;
    const title = window.prompt('Notebook title', notebook.title)?.trim();
    if (!title) return;
    try {
      setError(null);
      await api(`/api/notebooks/${notebookId}/rename`, { method: 'POST', body: JSON.stringify({ title }) });
      if (await refresh()) setStatus('Notebook renamed');
    } catch (reason) { showFailure(reason); }
  };

  const deleteNotebook = async () => {
    if (!notebook || notebookId === null || !window.confirm(`Delete “${notebook.title}” and its drafts?`)) return;
    try {
      setError(null); setStatus('Deleting notebook…');
      await api(`/api/notebooks/${notebookId}`, { method: 'DELETE' });
      generation.current += 1;
      setNotebookId(null); setNotebook(null); setCatalog([]); setDrafts([]); setPreview(null); resetAnswer();
      sessionStorage.removeItem('maestria.studio.notebook');
      queryClient.invalidateQueries({ queryKey: ['studio-bootstrap'] });
      setStatus('Notebook deleted');
    } catch (reason) { showFailure(reason); }
  };

  const toggleSource = async (source: CatalogSource, checked: boolean) => {
    if (notebookId === null) return;
    const token = generation.current;
    try {
      setError(null);
      await api(`/api/notebooks/${notebookId}/sources/${encodeURIComponent(source.source_key)}`, { method: checked ? 'POST' : 'DELETE' });
      if (token === generation.current && await refresh()) setStatus(checked ? 'Source attached' : 'Source detached');
    } catch (reason) { showFailure(reason); }
  };

  const ask = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!notebookId || !query.trim() || !agent || agent.status !== 'ready') return;
    const question = query.trim();
    const token = generation.current;
    try {
      setBusy(true); setError(null); setAnswerState('Working…'); setStatus('Asking selected sources…');
      const response = await api<{ answer_markdown: string; citations: Citation[]; draft_previews: { title: string; markdown: string; citations: Citation[] }[]; context: Context }>(`/api/notebooks/${notebookId}/ask`, {
        method: 'POST',
        body: JSON.stringify({ question, history, agent_id: agent.id, config: {} }),
      });
      if (token !== generation.current) return;
      setAnswer(response.answer_markdown);
      setContext(response.context);
      const nextHistory: { role: 'user' | 'assistant'; markdown: string }[] = [
        { role: 'user', markdown: question },
        { role: 'assistant', markdown: response.answer_markdown },
      ];
      setHistory((previous) => [...previous, ...nextHistory].slice(-12));
      setAnswerState('Unsaved answer');
      const firstPreview = response.draft_previews[0];
      setPreview({
        draftId: null,
        revision: null,
        title: firstPreview?.title ?? `Answer: ${question.slice(0, 120)}`,
        markdown: firstPreview?.markdown ?? response.answer_markdown,
        evidenceIds: (firstPreview?.citations ?? response.citations).map((citation) => citation.evidence.evidence_id),
      });
      setStatus('Answer ready; save a draft explicitly');
    } catch (reason) {
      if (token === generation.current) { setAnswerState('Unavailable'); showFailure(reason); }
    } finally { if (token === generation.current) setBusy(false); }
  };

  const saveDraft = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!notebookId || !preview) return;
    const form = new FormData(event.currentTarget);
    const title = form.get('title')?.toString().trim() ?? '';
    const markdown = form.get('markdown')?.toString() ?? '';
    try {
      setBusy(true); setError(null); setStatus('Saving draft…');
      const saved = await api<{ draft_id: number; revision: number }>(`/api/notebooks/${notebookId}/drafts`, { method: 'POST', body: JSON.stringify({ draft_id: preview.draftId, expected_revision: preview.revision, title, markdown, evidence_ids: preview.evidenceIds }) });
      if (await refresh()) {
        await loadDraft(saved.draft_id);
        setStatus(`Draft saved at revision ${saved.revision}`);
      }
    } catch (reason) { showFailure(reason); }
    finally { setBusy(false); }
  };

  const loadDraft = async (draftId: number) => {
    if (notebookId === null) return;
    try {
      setError(null); setStatus('Loading draft…');
      const draft = await api<Draft>(`/api/notebooks/${notebookId}/drafts/${draftId}`);
      setPreview({ draftId: draft.draft_id, revision: draft.revision, title: draft.title, markdown: draft.markdown, savedTitle: draft.title, savedMarkdown: draft.markdown, evidenceIds: draft.citations.map((citation) => citation.evidence_id) });
      setAnswer(draft.markdown); setAnswerState(`Saved draft revision ${draft.revision}`); setContext(null); setEvidence(null); setStatus('Draft loaded');
    } catch (reason) { showFailure(reason); }
  };

  const deleteDraft = async (draft: DraftSummary) => {
    if (notebookId === null || !window.confirm(`Delete draft “${draft.title}”?`)) return;
    try {
      await api(`/api/notebooks/${notebookId}/drafts/${draft.draft_id}`, { method: 'DELETE', body: JSON.stringify({ expected_revision: draft.revision }) });
      await refresh();
      if (preview?.draftId === draft.draft_id) setPreview(null);
      setStatus('Draft deleted');
    } catch (reason) { showFailure(reason); }
  };

  const openEvidence = async (evidenceId: number) => {
    if (notebookId === null) return;
    try { setEvidence(await api<Evidence>(`/api/notebooks/${notebookId}/evidence/${evidenceId}`)); setStatus('Evidence opened'); }
    catch (reason) { showFailure(reason); }
  };

  const selectedCount = notebook?.sources.filter((source) => source.available).length ?? 0;
  const availableCount = catalog.filter((source) => source.available).length;
  const coverage = context?.coverage;

  return <main>
    <header className="app-header"><div><p className="eyebrow">Local, source-grounded workspace</p><h1>Maestria Studio</h1><p id="agent">{agent ? `Agent: ${agent.label} (${agent.status})` : 'Agent: unconfigured'}</p></div><p id="status" className={`status ${error ? 'failure' : ''}`} role="status" aria-live="polite">{!notebook && notebooks.length === 0 ? 'Create a notebook to begin' : status}</p></header>
    {error && <div id="error" className="error" role="alert">{error.message}{error.code ? <small> ({error.code})</small> : null}</div>}
    <section className="layout">
      <aside className="sidebar" aria-labelledby="notebooks-heading"><div className="section-heading"><h2 id="notebooks-heading">Notebooks</h2><span id="notebook-count">{notebooks.length}</span></div><ul id="notebooks" className="notebook-list">{notebooks.length === 0 ? <li className="muted">No notebooks yet.</li> : notebooks.map((entry) => <li key={entry.notebook_id}><button type="button" className={entry.notebook_id === notebookId ? 'selected' : ''} aria-current={entry.notebook_id === notebookId ? 'page' : false} disabled={busy && entry.notebook_id === notebookId} onClick={() => void selectNotebook(entry.notebook_id)}>{entry.title}<small>{entry.source_count ?? 0} source{entry.source_count === 1 ? '' : 's'}</small></button></li>)}</ul><form id="create" className="stacked-form" onSubmit={(event) => void createNotebook(event)}><label htmlFor="new-title">New notebook</label><div className="inline-form"><input id="new-title" name="title" placeholder="Notebook title" required maxLength={120} /><button type="submit" disabled={busy}>Create</button></div></form></aside>
      <article className="workspace" aria-labelledby="notebook-title"><section className="workspace-header"><div><p className="eyebrow">Current notebook</p><h2 id="notebook-title">{notebook?.title ?? 'Select a notebook'}</h2><p id="notebook-meta" className="muted">{notebook ? `${selectedCount} selected source${selectedCount === 1 ? '' : 's'} · notebook ${notebook.notebook_id}` : 'No notebook selected.'}</p></div><div className="button-row"><button id="rename-notebook" type="button" disabled={!notebook || busy} onClick={() => void renameNotebook()}>Rename</button><button id="delete-notebook" type="button" className="danger" disabled={!notebook || busy} onClick={() => void deleteNotebook()}>Delete</button></div></section>
        <section className="panel" aria-labelledby="sources-heading"><div className="section-heading"><h3 id="sources-heading">Selected sources</h3><span id="source-count">{selectedCount}/{availableCount} selected</span></div><p className="muted">Only checked, available sources are included in questions.</p><ul id="sources" className="source-list">{!notebook ? <li className="muted">Select a notebook to manage sources.</li> : catalog.length === 0 ? <li className="muted">No indexed sources are available.</li> : catalog.map((source) => <li key={source.source_key} className={source.available ? '' : 'unavailable'}><label className="source-row"><input type="checkbox" checked={selectedSourceKeys.has(source.source_key)} disabled={!source.available || busy} aria-label={`Select ${source.title ?? source.source_key}`} onChange={(event) => void toggleSource(source, event.target.checked)} /><span><strong>{source.title ?? source.source_key}</strong><small>{source.source_key} · {source.index_status}{source.available ? '' : ' · unavailable'}</small></span></label></li>)}</ul></section>
        <section className="panel" aria-labelledby="ask-heading"><div className="section-heading"><h3 id="ask-heading">Ask grounded question</h3><span id="agent-options" className="muted">{agent?.config_options?.length ? `Options: ${agent.config_options.join(', ')}` : 'Default model configuration'}</span></div><form id="ask" className="stacked-form" onSubmit={(event) => void ask(event)}><label htmlFor="query">Question</label><textarea id="query" name="query" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Ask about the selected sources" required /><div className="button-row"><button id="ask-button" type="submit" disabled={busy || !notebook || agent?.status !== 'ready'}>Ask</button><button id="clear-answer" type="button" onClick={() => { generation.current += 1; setPreview(null); resetAnswer(); setStatus('Answer cleared'); }}>Clear</button></div></form></section>
        <section className="panel" aria-labelledby="answer-heading"><div className="section-heading"><h3 id="answer-heading">Answer</h3><span id="answer-state" className="muted">{answerState}</span></div><div id="answer" className="answer" tabIndex={0}><ReactMarkdown skipHtml>{answer}</ReactMarkdown></div>{coverage && <div id="coverage" className="coverage">{context?.answerability ?? 'unknown'} · {coverage.percent_covered ?? 0}% covered · {coverage.distinct_sources ?? 0} sources{context?.gaps?.length ? ` · Gaps: ${context.gaps.join('; ')}` : ''}</div>}<div id="citations" className="citation-list" aria-label="Citations">{context?.citations?.length ? <><h4>Citations</h4>{context.citations.map((citation) => <article className="citation" key={citation.evidence.evidence_id}><strong>#{citation.evidence.evidence_id} · {citation.evidence.artifact_title}</strong><p>{citation.evidence.excerpt}</p><small>Rank {citation.rank} · score {citation.score} · {sourceDescription(citation.evidence.source)} · hash {citation.evidence.artifact_content_hash ?? 'unavailable'}</small><button type="button" onClick={() => void openEvidence(citation.evidence.evidence_id)}>Open evidence</button></article>)}</> : <p className="muted">No citations were returned.</p>}</div></section>
        {preview && <section id="draft-panel" className="panel" aria-labelledby="draft-heading"><div className="section-heading"><h3 id="draft-heading">Draft preview</h3><span id="draft-state" className="unsaved">{preview.draftId === null ? 'Unsaved preview' : `Saved revision ${preview.revision}; edits are unsaved`}</span></div><form id="draft-form" className="stacked-form" onSubmit={(event) => void saveDraft(event)}><label htmlFor="draft-title">Draft title</label><input id="draft-title" name="title" defaultValue={preview.title} maxLength={160} required /><label htmlFor="draft-markdown">Markdown</label><textarea id="draft-markdown" name="markdown" className="markdown-editor" defaultValue={preview.markdown} required /><div className="button-row"><button id="save-draft" type="submit" disabled={busy}>Save draft</button><button id="discard-draft" type="button" onClick={() => { setPreview(null); setStatus('Unsaved preview discarded'); }}>Discard preview</button>{preview.draftId !== null && <button id="export-draft" type="button" onClick={() => { const blob = new Blob([preview.savedMarkdown ?? preview.markdown], { type: 'text/markdown;charset=utf-8' }); const link = document.createElement('a'); link.href = URL.createObjectURL(blob); link.download = `${preview.savedTitle ?? preview.title}.md`; link.click(); URL.revokeObjectURL(link.href); setStatus('Saved draft exported'); }}>Export saved Markdown</button>}</div></form></section>}
        <section className="panel" aria-labelledby="drafts-heading"><div className="section-heading"><h3 id="drafts-heading">Saved drafts</h3><span id="draft-count">{drafts.length}</span></div><ul id="drafts" className="draft-list">{!notebook ? <li className="muted">Select a notebook to view drafts.</li> : drafts.length === 0 ? <li className="muted">No saved drafts. Ask a question, then save explicitly.</li> : drafts.map((draft) => <li className="draft-row" key={draft.draft_id}><button type="button" onClick={() => void loadDraft(draft.draft_id)}>{draft.title} · revision {draft.revision}</button><button type="button" className="danger" onClick={() => void deleteDraft(draft)}>Delete</button></li>)}</ul></section>
        {evidence && <section id="evidence-panel" className="panel" aria-labelledby="evidence-heading"><div className="section-heading"><h3 id="evidence-heading">Evidence #{evidence.evidence_id}: {evidence.artifact_title}</h3><button id="close-evidence" type="button" onClick={() => setEvidence(null)}>Close</button></div><dl><dt>Artifact</dt><dd>{evidence.artifact_id}</dd><dt>Content hash</dt><dd>{evidence.artifact_content_hash ?? 'unavailable'}</dd><dt>Observed at</dt><dd>{evidence.observed_at}</dd></dl><h4>Excerpt</h4><pre className="evidence-excerpt">{evidence.excerpt}</pre></section>}
      </article>
    </section>
  </main>;
}
const root = document.getElementById('root');
if (root) {
  createRoot(root).render(<QueryClientProvider client={queryClient}><App /></QueryClientProvider>);
}

