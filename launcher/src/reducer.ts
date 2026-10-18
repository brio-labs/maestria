import type {
  ActionOutcome,
  FileSelection,
  LauncherState,
  PreferencesDto,
  SearchResponse,
  ShortcutStatus,
} from './model';
import { initialState, selectedResult } from './model';

export type LauncherAction =
  | { type: 'connection'; connection: LauncherState['connection'] }
  | { type: 'query'; query: string }
  | { type: 'search-response'; response: SearchResponse }
  | { type: 'search-error'; generation: number; message: string }
  | { type: 'catalog-changed'; revision: number }
  | { type: 'activate'; generation: number; preferences: PreferencesDto; metricsEnabled: boolean }
  | { type: 'select'; id: string }
  | { type: 'move-selection'; delta: -1 | 1 }
  | { type: 'toggle-actions' }
  | { type: 'close-actions' }
  | { type: 'move-action'; delta: -1 | 1 }
  | { type: 'select-action'; index: number }
  | { type: 'action-start' }
  | { type: 'action-outcome'; outcome: ActionOutcome }
  | { type: 'action-error'; message: string }
  | { type: 'dismiss' }
  | { type: 'dismiss-acknowledged' }
  | { type: 'escape' }
  | { type: 'open-preferences' }
  | { type: 'preferences-loading' }
  | { type: 'preferences-loaded'; preferences: PreferencesDto }
  | { type: 'preferences-error'; message: string }
  | { type: 'shortcut-status'; status: ShortcutStatus }
  | { type: 'reset-confirmation'; value: boolean };

function retainSelection(selectedId: string | null, results: SearchResponse['results']): string | null {
  if (selectedId && results.some((result) => result.id === selectedId)) return selectedId;
  return results[0]?.id ?? null;
}

function actionCount(state: LauncherState): number {
  return selectedResult(state)?.actions.length ?? 0;
}

export function launcherReducer(state: LauncherState, action: LauncherAction): LauncherState {
  switch (action.type) {
    case 'connection':
      return { ...state, connection: action.connection };
    case 'query': {
      const generation = state.generation + 1;
      return {
        ...state,
        view: 'root',
        query: action.query,
        generation,
        resultsGeneration: null,
        results: [],
        actionPanelOpen: false,
        actionPanelIndex: 0,
        actionError: null,
        fileSelection: null,
        copyNotice: false,
        status: { kind: 'loading', message: null },
      };
    }
    case 'search-response':
      if (action.response.generation !== state.generation || action.response.catalogRevision < state.catalogRevision) return state;
      return {
        ...state,
        catalogRevision: action.response.catalogRevision,
        resultsGeneration: action.response.generation,
        results: action.response.results,
        status: action.response.status,
        copyNotice: false,
        selectedId: retainSelection(state.selectedId, action.response.results),
      };
    case 'search-error':
      if (action.generation !== state.generation) return state;
      return { ...state, status: { kind: 'error', message: action.message } };
    case 'catalog-changed':
      if (action.revision <= state.catalogRevision) return state;
      // Do not start a same-generation search while a native effect owns the
      // current result set. Its completion must still be accepted, and the
      // next root search will observe the refreshed catalog snapshot.
      if (state.actionInFlight || state.fileSelection || state.view !== 'root') return state;
      return { ...state, generation: state.generation + 1, catalogRevision: action.revision,
        resultsGeneration: null, results: [], actionPanelOpen: false, actionPanelIndex: 0,
        copyNotice: false, status: { kind: 'refreshing', message: null } };
    case 'activate': {
      const generation = Math.max(action.generation + 1, state.generation + 1);
      return {
        ...state,
        view: 'root',
        query: '',
        generation,
        activationGeneration: action.generation,
        metricsEnabled: action.metricsEnabled,
        resultsGeneration: null,
        preferences: action.preferences,
        preferencesError: null,
        results: [],
        status: { kind: 'loading', message: null },
        selectedId: null,
        actionPanelOpen: false,
        actionPanelIndex: 0,
        actionError: null,
        copyNotice: false,
        fileSelection: null,
        resetConfirmation: false,
        dismissed: false,
        activationFocus: state.activationFocus + 1,
      };
    }
    case 'select':
      if (!state.results.some((result) => result.id === action.id) && action.id !== state.fileSelection?.resultId) return state;
      return { ...state, selectedId: action.id };
    case 'move-selection': {
      if (state.actionPanelOpen || state.results.length === 0) return state;
      const current = Math.max(0, state.results.findIndex((result) => result.id === state.selectedId));
      const next = (current + action.delta + state.results.length) % state.results.length;
      return { ...state, selectedId: state.results[next]?.id ?? null };
    }
    case 'toggle-actions':
      if (!selectedResult(state) || actionCount(state) === 0) return state;
      return { ...state, actionPanelOpen: !state.actionPanelOpen, actionPanelIndex: 0 };
    case 'close-actions':
      return { ...state, actionPanelOpen: false, actionPanelIndex: 0, activationFocus: state.activationFocus + 1 };
    case 'select-action':
      if (action.index < 0 || action.index >= actionCount(state)) return state;
      return { ...state, actionPanelIndex: action.index };
    case 'move-action': {
      const count = actionCount(state);
      if (!state.actionPanelOpen || count === 0) return state;
      const next = (state.actionPanelIndex + action.delta + count) % count;
      return { ...state, actionPanelIndex: next };
    }
    case 'action-start':
      return { ...state, actionInFlight: true, actionError: null, copyNotice: false };
    case 'action-outcome': {
      const { outcome } = action;
      const fileSelection: FileSelection | null = outcome.kind === 'file_selected'
        ? { resultId: outcome.resultId, displayPath: outcome.displayPath }
        : state.fileSelection;
      if (outcome.kind === 'show_preferences') {
        return { ...state, actionInFlight: false, actionPanelOpen: false, view: 'preferences', fileSelection, actionError: null };
      }
      return {
        ...state,
        actionInFlight: false,
        actionPanelOpen: false,
        selectedId: outcome.kind === 'file_selected' ? outcome.resultId : state.selectedId,
        results: outcome.kind === 'file_selected' ? [] : state.results,
        fileSelection,
        actionError: null,
        copyNotice: outcome.kind === 'copied',
      };
    }
    case 'action-error':
      return { ...state, actionInFlight: false, actionError: action.message, copyNotice: false };
    case 'dismiss':
      return { ...state, dismissed: true, actionPanelOpen: false };
    case 'dismiss-acknowledged':
      return { ...state, dismissed: false };
    case 'escape':
      if (state.actionPanelOpen) return { ...state, actionPanelOpen: false, actionPanelIndex: 0 };
      if (state.view === 'preferences') {
        if (state.resetConfirmation) return { ...state, resetConfirmation: false };
        return {
          ...state,
          view: 'root',
          fileSelection: null,
          results: [],
          generation: state.generation + 1,
          resultsGeneration: null,
          status: { kind: 'loading', message: null },
          actionError: null,
          resetConfirmation: false,
          activationFocus: state.activationFocus + 1,
        };
      }
      if (state.fileSelection) {
        return { ...state, fileSelection: null, results: [], selectedId: null, generation: state.generation + 1,
          resultsGeneration: null, actionError: null, status: { kind: 'loading', message: null },
          activationFocus: state.activationFocus + 1 };
      }
      return { ...state, dismissed: true };
    case 'open-preferences':
      return { ...state, view: 'preferences', actionPanelOpen: false, resetConfirmation: false, preferencesError: null };
    case 'preferences-loading':
      return { ...state, preferencesLoading: true, preferencesError: null };
    case 'preferences-loaded':
      return { ...state, preferencesLoading: false, preferencesError: null, preferences: action.preferences };
    case 'preferences-error':
      return { ...state, preferencesLoading: false, preferencesError: action.message };
    case 'shortcut-status':
      if (!state.preferences) return state;
      return { ...state, preferences: { ...state.preferences, shortcutStatus: action.status } };
    case 'reset-confirmation':
      return { ...state, resetConfirmation: action.value };
    default:
      return state;
  }
}

export { initialState };
