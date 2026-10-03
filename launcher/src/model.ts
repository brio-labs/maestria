export type ResultKind = 'application' | 'command' | 'calculation';

export type Action = {
  id: string;
  title: string;
  primary: boolean;
};

export type Result = {
  id: string;
  kind: ResultKind;
  title: string;
  subtitle: string | null;
  icon: string | null;
  actions: Action[];
};

export type StatusKind = 'loading' | 'ready' | 'refreshing' | 'error' | 'calculation_error';

export type SearchStatus = {
  kind: StatusKind;
  message: string | null;
};

export type SearchResponse = {
  generation: number;
  catalogRevision: number;
  results: Result[];
  status: SearchStatus;
};

export type ActionOutcome =
  | { kind: 'dismiss' | 'copied' | 'show_preferences' | 'refreshed' | 'cancelled'; resultId?: string; displayPath?: string }
  | { kind: 'file_selected'; resultId: string; displayPath: string };

export type LauncherError = {
  code: string;
  message: string;
  recoverable: boolean;
};

export type ShortcutStatus = {
  state: 'unconfigured' | 'available' | 'unavailable';
  description: string;
  message: string | null;
  control: 'application' | 'system' | 'unavailable';
  configureAction: 'setup' | 'change' | 'rebind' | 'retry';
};

export type ShortcutSetup = 'unconfigured' | 'requested' | 'deferred';

export type PreferencesDto = {
  schemaVersion: 1;
  shortcut: string;
  shortcutSetup: ShortcutSetup;
  reduceMotion: boolean;
  warning: string | null;
  readOnly: boolean;
  platform: string;
  shortcutStatus: ShortcutStatus;
  accelerators: { primaryModifier: 'control' | 'meta'; primaryLabel: string };
};

export type PreferencesUpdate = {
  reduceMotion?: boolean;
  shortcut?: string;
  shortcutSetup?: ShortcutSetup;
  confirmReset?: boolean;
};

export type LauncherEventMap = {
  'launcher://activate': { generation: number; preferences: PreferencesDto; metricsEnabled: boolean };
  'launcher://catalog-changed': { revision: number };
  'launcher://shortcut-status': ShortcutStatus;
};

export type View = 'root' | 'preferences';

export type FileSelection = {
  resultId: string;
  displayPath: string;
};

export type LauncherState = {
  view: View;
  query: string;
  generation: number;
  catalogRevision: number;
  results: Result[];
  status: SearchStatus;
  selectedId: string | null;
  actionPanelOpen: boolean;
  actionPanelIndex: number;
  actionInFlight: boolean;
  actionError: string | null;
  copyNotice: boolean;
  fileSelection: FileSelection | null;
  preferences: PreferencesDto | null;
  preferencesLoading: boolean;
  preferencesError: string | null;
  resetConfirmation: boolean;
  connection: 'connecting' | 'ready' | 'unavailable';
  dismissed: boolean;
  activationGeneration: number | null;
  metricsEnabled: boolean;
  resultsGeneration: number | null;
  activationFocus: number;
};

export const initialState: LauncherState = {
  view: 'root',
  query: '',
  generation: 0,
  catalogRevision: 0,
  results: [],
  status: { kind: 'loading', message: null },
  selectedId: null,
  actionPanelOpen: false,
  actionPanelIndex: 0,
  actionInFlight: false,
  actionError: null,
  copyNotice: false,
  fileSelection: null,
  preferences: null,
  preferencesLoading: false,
  preferencesError: null,
  resetConfirmation: false,
  connection: 'connecting',
  dismissed: false,
  activationFocus: 0,
  activationGeneration: null,
  metricsEnabled: false,
  resultsGeneration: null,
};

export function selectedResult(state: LauncherState): Result | null {
  if (state.fileSelection) {
    return {
      id: state.fileSelection.resultId,
      kind: 'command',
      title: state.fileSelection.displayPath,
      subtitle: 'Selected file',
      icon: null,
      actions: [
        { id: 'file.open', title: 'Open', primary: true },
        { id: 'file.copy-path', title: 'Copy Path', primary: false },
      ],
    };
  }
  return state.results.find((result) => result.id === state.selectedId) ?? null;
}

export function primaryAction(result: Result | null): Action | null {
  return result?.actions.find((action) => action.primary) ?? result?.actions[0] ?? null;
}
