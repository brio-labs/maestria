import { describe, expect, it } from 'vitest';
import { initialState, selectedResult, type LauncherState, type Result } from '../../src/model';
import { launcherReducer } from '../../src/reducer';

const app: Result = {
  id: 'app:editor',
  kind: 'application',
  title: 'Editor',
  subtitle: 'Edit text',
  icon: null,
  actions: [{ id: 'app.open', title: 'Open', primary: true }],
};

const terminal: Result = {
  id: 'app:terminal',
  kind: 'application',
  title: 'Terminal',
  subtitle: null,
  icon: null,
  actions: [{ id: 'app.open', title: 'Open', primary: true }],
};

describe('launcher reducer', () => {
  it('retains a selected result when a catalog refresh preserves its ID', () => {
    let state = launcherReducer(initialState, { type: 'query', query: 'e' });
    state = launcherReducer(state, {
      type: 'search-response',
      response: { generation: state.generation, catalogRevision: 1, results: [app, terminal], status: { kind: 'ready', message: null } },
    });
    state = launcherReducer(state, { type: 'select', id: 'app:terminal' });
    state = launcherReducer(state, { type: 'catalog-changed', revision: 2 });
    expect(state.results).toEqual([]);
    expect(selectedResult(state)).toBeNull();
    expect(state.status.kind).toBe('refreshing');
    state = launcherReducer(state, {
      type: 'search-response',
      response: { generation: state.generation, catalogRevision: 2, results: [terminal, app], status: { kind: 'ready', message: null } },
    });
    expect(state.selectedId).toBe('app:terminal');
  });

  it('keeps catalog failures visible when an action copies a value', () => {
    const catalogError = { kind: 'error' as const, message: 'Application catalog unavailable' };
    const state = launcherReducer({
      ...initialState, results: [app], selectedId: app.id, status: catalogError,
    }, { type: 'action-outcome', outcome: { kind: 'copied' } });
    expect(state.status).toEqual(catalogError);
    expect(state.copyNotice).toBe(true);
  });

  it('defers catalog refresh while a native action or selected file owns the generation', () => {
    const pending = { ...initialState, generation: 7, catalogRevision: 3, actionInFlight: true };
    const duringAction = launcherReducer(pending, { type: 'catalog-changed', revision: 4 });
    expect(duringAction).toEqual(pending);

    const selected = launcherReducer(duringAction, {
      type: 'action-outcome',
      outcome: { kind: 'file_selected', resultId: 'file:opaque', displayPath: '/tmp/selected.txt' },
    });
    const acceptedGeneration = selected.generation;
    const duringSelectedFile = launcherReducer(selected, { type: 'catalog-changed', revision: 5 });
    expect(duringSelectedFile.generation).toBe(acceptedGeneration);
    expect(duringSelectedFile.catalogRevision).toBe(3);
    expect(duringSelectedFile.fileSelection).toEqual(selected.fileSelection);
  });

  it('ignores stale responses after a newer query generation', () => {
    let state = launcherReducer(initialState, { type: 'query', query: 'old' });
    const oldGeneration = state.generation;
    state = launcherReducer(state, { type: 'query', query: 'new' });
    state = launcherReducer(state, {
      type: 'search-response',
      response: { generation: oldGeneration, catalogRevision: 1, results: [app], status: { kind: 'ready', message: null } },
    });
    expect(state.results).toEqual([]);
    expect(state.generation).toBe(oldGeneration + 1);
  });


  it('closes actions, returns from preferences, then dismisses the root in Escape layers', () => {
    let state = launcherReducer({ ...initialState, results: [app], selectedId: app.id, actionPanelOpen: true }, { type: 'escape' });
    expect(state.actionPanelOpen).toBe(false);
    state = launcherReducer({ ...state, view: 'preferences',
      fileSelection: { resultId: 'file:opaque', displayPath: '/tmp/selected.txt' } }, { type: 'escape' });
    expect(state.view).toBe('root');
    expect(state.dismissed).toBe(false);
    expect(selectedResult(state)).toBeNull();
    state = launcherReducer(state, { type: 'escape' });
    expect(state.dismissed).toBe(true);
  });

  it('retains selection across changed queries only while the result still matches', () => {
    let state: LauncherState = { ...initialState, results: [app, terminal], selectedId: terminal.id };
    state = launcherReducer(state, { type: 'query', query: 'term' });
    state = launcherReducer(state, { type: 'search-response', response: {
      generation: state.generation, catalogRevision: 1, results: [terminal], status: { kind: 'ready', message: null },
    } });
    expect(state.selectedId).toBe(terminal.id);
    state = launcherReducer(state, { type: 'query', query: 'editor' });
    state = launcherReducer(state, { type: 'search-response', response: {
      generation: state.generation, catalogRevision: 1, results: [app], status: { kind: 'ready', message: null },
    } });
    expect(state.selectedId).toBe(app.id);
  });

  it('removes a copyable calculation immediately when its expression changes', () => {
    const calculation: Result = { ...app, id: 'calculation', kind: 'calculation', title: '4',
      actions: [{ id: 'calculation.copy', title: 'Copy Result', primary: true }] };
    const state = launcherReducer({ ...initialState, results: [calculation], selectedId: calculation.id },
      { type: 'query', query: '2 +' });
    expect(state.results).toEqual([]);
  });

  it('returns a selected file to root before dismissing', () => {
    let state = launcherReducer(initialState, { type: 'action-outcome',
      outcome: { kind: 'file_selected', resultId: 'file:opaque', displayPath: '/tmp/selected.txt' } });
    state = launcherReducer(state, { type: 'escape' });
    expect(state.fileSelection).toBeNull();
    expect(state.dismissed).toBe(false);
    state = launcherReducer(state, { type: 'escape' });
    expect(state.dismissed).toBe(true);
  });
});
