import { useEffect, useReducer, useRef, useState } from 'react';
import {
  configureShortcut,
  dismiss,
  errorMessage,
  frameReady,
  executeAction,
  getPreferences,
  listenLauncherEvents,
  savePreferences,
  search,
  signalLauncherReady,
} from './bridge';
import markAsset from './assets/maestria-mark.svg';
import searchAsset from './assets/search.svg';
import { ActionPanel } from './components/ActionPanel';
import { Preferences } from './components/Preferences';
import { ResultList } from './components/ResultList';
import { SearchInput } from './components/SearchInput';
import { StatusMessage } from './components/StatusMessage';
import { initialState, primaryAction, selectedResult, type Action, type LauncherEventMap, type PreferencesUpdate, type ShortcutStatus } from './model';
import { launcherReducer } from './reducer';
import { getCurrentWindow } from '@tauri-apps/api/window';

export function App() {
  const [state, dispatch] = useReducer(launcherReducer, initialState);
  const [preferenceActionInFlight, setPreferenceActionInFlight] = useState(false);
  const [preferenceNotice, setPreferenceNotice] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const stateRef = useRef(state);
  const readyRef = useRef(false);
  const pendingActivationRef = useRef<LauncherEventMap['launcher://activate'] | null>(null);
  const pendingShortcutRef = useRef<ShortcutStatus | null>(null);
  const actionInFlightRef = useRef(false);
  const searchRequestRef = useRef<string | null>(null);
  const preferencesRequestRef = useRef<string | null>(null);
  const composingRef = useRef(false);
  const panelWasOpenRef = useRef(false);
  const activationClockRef = useRef<{ generation: number; started: number } | null>(null);
  const resultsClockRef = useRef<{ generation: number; started: number } | null>(null);

  stateRef.current = state;

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;

    const register = listenLauncherEvents({
      'launcher://activate': (payload) => {
        if (payload.metricsEnabled) {
          activationClockRef.current = { generation: payload.generation, started: performance.now() };
        }
        if (!readyRef.current) {
          pendingActivationRef.current = payload;
          pendingShortcutRef.current = null;
          return;
        }
        dispatch({ type: 'activate', ...payload });
      },
      'launcher://catalog-changed': (payload) => {
        dispatch({ type: 'catalog-changed', revision: payload.revision });
      },
      'launcher://shortcut-status': (payload) => {
        if (!readyRef.current) pendingShortcutRef.current = payload;
        else dispatch({ type: 'shortcut-status', status: payload });
      },
    });

    void register.then(async (cleanup) => {
      if (!active) {
        cleanup();
        return;
      }
      unlisten = cleanup;
      await signalLauncherReady();
      if (!active) return;
      readyRef.current = true;
      dispatch({ type: 'connection', connection: 'ready' });
      const pending = pendingActivationRef.current;
      pendingActivationRef.current = null;
      if (pending) dispatch({ type: 'activate', ...pending });
      if (pendingShortcutRef.current) dispatch({ type: 'shortcut-status', status: pendingShortcutRef.current });
      pendingShortcutRef.current = null;
    }).catch((error: unknown) => {
      if (!active) return;
      dispatch({ type: 'connection', connection: 'unavailable' });
      dispatch({ type: 'search-error', generation: stateRef.current.generation, message: errorMessage(error) });
    });

    return () => {
      active = false;
      readyRef.current = false;
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    if (state.connection !== 'ready' || state.view !== 'root') return;
    const key = `${state.generation}:${state.catalogRevision}`;
    if (searchRequestRef.current === key) return;
    if (state.metricsEnabled && resultsClockRef.current?.generation !== state.generation) {
      resultsClockRef.current = { generation: state.generation, started: performance.now() };
    }
    searchRequestRef.current = key;
    void search(state.query, state.generation).then((response) => {
      searchRequestRef.current = `${response.generation}:${response.catalogRevision}`;
      dispatch({ type: 'search-response', response });
    }).catch((error: unknown) => {
      dispatch({ type: 'search-error', generation: state.generation, message: errorMessage(error) });
    });
  }, [state.connection, state.view, state.query, state.generation, state.catalogRevision]);
  useEffect(() => {
    const generation = state.activationGeneration;
    const clock = activationClockRef.current;
    if (!state.metricsEnabled || generation === null || clock?.generation !== generation) return;
    const frame = requestAnimationFrame(() => {
      if (activationClockRef.current?.generation !== generation) return;
      activationClockRef.current = null;
      void frameReady(generation, 'activation', performance.now() - clock.started).catch(() => {});
    });
    return () => cancelAnimationFrame(frame);
  }, [state.activationGeneration, state.metricsEnabled]);

  useEffect(() => {
    const generation = state.resultsGeneration;
    const clock = resultsClockRef.current;
    if (!state.metricsEnabled || generation === null || generation !== state.generation
        || clock?.generation !== generation || state.view !== 'root') return;
    const frame = requestAnimationFrame(() => {
      if (resultsClockRef.current?.generation !== generation) return;
      resultsClockRef.current = null;
      void frameReady(generation, 'results', performance.now() - clock.started).catch(() => {});
    });
    return () => cancelAnimationFrame(frame);
  }, [state.resultsGeneration, state.generation, state.metricsEnabled, state.view]);


  useEffect(() => {
    if (state.view !== 'preferences') return;
    const key = `${state.view}:${state.generation}`;
    if (preferencesRequestRef.current === key) return;
    preferencesRequestRef.current = key;
    dispatch({ type: 'preferences-loading' });
    void getPreferences().then((preferences) => {
      dispatch({ type: 'preferences-loaded', preferences });
    }).catch((error: unknown) => {
      dispatch({ type: 'preferences-error', message: errorMessage(error) });
    });
  }, [state.view, state.generation]);

  useEffect(() => {
    if (state.activationFocus === 0 || state.view !== 'root') return;
    inputRef.current?.focus();
  }, [state.activationFocus, state.view]);

  useEffect(() => {
    if (panelWasOpenRef.current && !state.actionPanelOpen) inputRef.current?.focus();
    panelWasOpenRef.current = state.actionPanelOpen;
  }, [state.actionPanelOpen]);

  useEffect(() => {
    // Disabling an in-flight button can move focus to body. Launcher-wide
    // shortcuts must still work without forcibly moving the user's focus.
    function handleKey(event: KeyboardEvent) {
      if (event.defaultPrevented || composingRef.current || event.isComposing) return;
      const current = stateRef.current;
      const modifier = current.preferences?.accelerators.primaryModifier;
      const primaryPressed = modifier === 'control' ? event.ctrlKey : modifier === 'meta' && event.metaKey;
      if (event.key === 'Escape') {
        event.preventDefault();
        dispatch({ type: 'escape' });
      } else if (primaryPressed && event.key === ',') {
        event.preventDefault();
        dispatch({ type: 'open-preferences' });
        setPreferenceNotice(null);
      } else if (primaryPressed && event.key.toLowerCase() === 'k' && current.view === 'root') {
        event.preventDefault();
        dispatch({ type: 'toggle-actions' });
      }
    }
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, []);

  useEffect(() => {
    if (!state.dismissed) return;
    dispatch({ type: 'dismiss-acknowledged' });
    void dismiss().catch((error: unknown) => {
      dispatch({ type: 'action-error', message: errorMessage(error) });
    });
  }, [state.dismissed]);
  function handleSubmit(actionOverride?: Action) {
    const current = stateRef.current;
    if (actionInFlightRef.current || current.actionInFlight || current.view !== 'root') return;
    const result = selectedResult(current);
    const action = actionOverride ?? primaryAction(result);
    if (!result || !action) return;
    actionInFlightRef.current = true;
    dispatch({ type: 'action-start' });
    void executeAction(result.id, action.id, current.generation).then((outcome) => {
      actionInFlightRef.current = false;
      dispatch({ type: 'action-outcome',
        outcome: stateRef.current.generation === current.generation ? outcome : { kind: 'cancelled' } });
    }).catch((error: unknown) => {
      actionInFlightRef.current = false;
      if (stateRef.current.generation === current.generation) {
        dispatch({ type: 'action-error', message: errorMessage(error) });
      } else {
        dispatch({ type: 'action-outcome', outcome: { kind: 'cancelled' } });
      }
    });
  }

  function openPreferences() {
    dispatch({ type: 'open-preferences' });
    setPreferenceNotice(null);
  }

  function handlePreferenceSave(request: PreferencesUpdate) {
    if (preferenceActionInFlight) return;
    setPreferenceActionInFlight(true);
    setPreferenceNotice(null);
    void savePreferences(request).then((preferences) => {
      dispatch({ type: 'preferences-loaded', preferences });
      setPreferenceActionInFlight(false);
      setPreferenceNotice(preferences.warning ? 'Preferences applied for this session' : 'Preferences saved');
    }).catch((error: unknown) => {
      setPreferenceActionInFlight(false);
      dispatch({ type: 'preferences-error', message: errorMessage(error) });
    });
  }

  function handleConfigureShortcut() {
    if (preferenceActionInFlight) return;
    setPreferenceActionInFlight(true);
    setPreferenceNotice(null);
    void configureShortcut().then(async (status) => {
      dispatch({ type: 'shortcut-status', status });
      const preferences = await getPreferences();
      dispatch({ type: 'preferences-loaded', preferences });
      setPreferenceNotice(!status.message && status.state === 'available' ? 'Shortcut configured' : null);
    }).catch((error: unknown) => {
      dispatch({ type: 'preferences-error', message: errorMessage(error) });
    }).finally(() => setPreferenceActionInFlight(false));
  }

  function handleCopyActivation() {
    if (preferenceActionInFlight) return;
    setPreferenceActionInFlight(true);
    setPreferenceNotice(null);
    void executeAction('host.preferences', 'host.copy-activation', stateRef.current.generation).then((outcome) => {
      setPreferenceActionInFlight(false);
      setPreferenceNotice(outcome.kind === 'copied' ? 'Copied' : outcome.kind === 'cancelled' ? 'Cancelled' : outcome.kind);
    }).catch((error: unknown) => {
      setPreferenceActionInFlight(false);
      dispatch({ type: 'preferences-error', message: errorMessage(error) });
    });
  }
  function handleResetPreferences() {
    if (preferenceActionInFlight) return;
    setPreferenceActionInFlight(true);
    setPreferenceNotice(null);
    void savePreferences({ confirmReset: true }).then((preferences) => {
      dispatch({ type: 'preferences-loaded', preferences });
      return executeAction('host.preferences', 'host.reset-preferences', stateRef.current.generation);
    }).then(() => {
      return getPreferences();
    }).then((preferences) => {
      dispatch({ type: 'preferences-loaded', preferences });
      setPreferenceActionInFlight(false);
      setPreferenceNotice('Preferences reset');
    }).catch((error: unknown) => {
      setPreferenceActionInFlight(false);
      dispatch({ type: 'preferences-error', message: errorMessage(error) });
    });
  }

  async function startDrag() {
    try {
      await getCurrentWindow().startDragging();
    } catch {
      // Window managers that do not expose dragging simply ignore this region.
    }
  }

  const selected = selectedResult(state);
  const panelActions = selected?.actions ?? [];

  return (
    <main className="launcher-window" aria-label="Sillage Launcher"
      onCompositionStartCapture={() => { composingRef.current = true; }}
      onCompositionEndCapture={() => { composingRef.current = false; }}>
      <section className={`launcher-shell${state.preferences?.reduceMotion ? ' reduce-motion' : ''}`}>
        <div className="drag-region" aria-hidden="true" onMouseDown={(event) => { if (event.button === 0) void startDrag(); }} />
        <header className="search-header">
          <img className="search-icon" src={searchAsset} alt="" aria-hidden="true" />
          <SearchInput
            ref={inputRef}
            query={state.query}
            selectedId={selected?.id ?? null}
            disabled={state.connection !== 'ready' || state.view === 'preferences'}
            onChange={(query) => {
              if (stateRef.current.metricsEnabled) {
                resultsClockRef.current = { generation: stateRef.current.generation + 1, started: performance.now() };
              }
              dispatch({ type: 'query', query });
            }}
            onMove={(delta) => dispatch({ type: 'move-selection', delta })}
            onSubmit={() => handleSubmit()}
            onEscape={() => dispatch({ type: 'escape' })}
          />
          <span className="connection-dot" data-status={state.connection} aria-label={state.connection} />
        </header>
        <div className="result-surface">
          {state.view === 'preferences' ? (
            <Preferences
              preferences={state.preferences}
              loading={state.preferencesLoading}
              busy={preferenceActionInFlight}
              error={state.preferencesError}
              notice={preferenceNotice}
              resetConfirmation={state.resetConfirmation}
              onBack={() => dispatch({ type: 'escape' })}
              onSave={handlePreferenceSave}
              onConfigure={handleConfigureShortcut}
              onCopyActivation={handleCopyActivation}
              onReset={handleResetPreferences}
              onResetConfirmation={(value) => dispatch({ type: 'reset-confirmation', value })}
            />
          ) : (
            <>
              {state.preferences?.shortcutSetup === 'unconfigured' && !state.preferences.readOnly ? (
                <section className="shortcut-offer" aria-label="Shortcut setup">
                  <p>Open Sillage from anywhere with a keyboard shortcut.</p>
                  <div className="shortcut-actions">
                    <button type="button" className="primary-button"
                      disabled={preferenceActionInFlight || state.preferences.shortcutStatus.control === 'unavailable'}
                      onClick={() => { openPreferences(); handleConfigureShortcut(); }}>Set Up Shortcut</button>
                    <button type="button" className="secondary-button" disabled={preferenceActionInFlight}
                      onClick={() => handlePreferenceSave({ shortcutSetup: 'deferred' })}>Not Now</button>
                  </div>
                </section>
              ) : null}
              {state.preferences?.warning ? <p className="preference-warning" role="status">{state.preferences.warning}</p> : null}
              {state.preferencesError ? <p className="preference-error" role="alert">{state.preferencesError}</p> : null}
              {state.actionInFlight ? <div className="status-message" role="status">Working…</div> : null}
              <StatusMessage
                status={state.status}
                query={state.query}
                resultCount={state.results.length + (state.fileSelection ? 1 : 0)}
                actionError={state.actionError}
                onClear={() => { dispatch({ type: 'query', query: '' }); inputRef.current?.focus(); }}
              />
              {state.copyNotice ? <div className="status-message" role="status" aria-live="polite">Copied</div> : null}
              {state.results.length || state.fileSelection ? (
                <ResultList
                  results={state.results}
                  selectedId={state.selectedId}
                  fileSelection={state.fileSelection}
                  actionInFlight={state.actionInFlight}
                  onSelect={(id) => dispatch({ type: 'select', id })}
                  onSubmit={() => handleSubmit()}
                />
              ) : null}
            </>
          )}
        </div>

        {state.view === 'root' && state.actionPanelOpen ? (
          <ActionPanel
            actions={panelActions}
            selectedIndex={state.actionPanelIndex}
            disabled={state.actionInFlight}
            onMove={(delta) => dispatch({ type: 'move-action', delta })}
            onSelect={(index) => dispatch({ type: 'select-action', index })}
            onExecute={(action) => handleSubmit(action)}
            onClose={() => dispatch({ type: 'close-actions' })}
          />
        ) : null}

        <footer className="launcher-footer">
          <div className="footer-brand">
            <img src={markAsset} alt="" aria-hidden="true" />
            <span>Sillage</span>
          </div>
          <div className="footer-actions">
            {state.view === 'root' ? (
              <>
                <button type="button" className="footer-button" disabled={!selected || state.actionInFlight} onClick={() => handleSubmit()}>
                  {state.actionInFlight ? 'Working…' : primaryAction(selected)?.title ?? 'Select an action'} <span className="keycap">Enter</span>
                </button>
                <button type="button" className="footer-button" disabled={!selected || state.actionInFlight} onClick={() => dispatch({ type: 'toggle-actions' })}>
                  Actions <span className="keycap">{state.preferences?.accelerators.primaryLabel} K</span>
                </button>
              </>
            ) : (
              <button type="button" className="footer-button" onClick={() => dispatch({ type: 'escape' })}>Back <span className="keycap">Esc</span></button>
            )}
          </div>
        </footer>
      </section>
    </main>
  );
}
