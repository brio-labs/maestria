import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  ActionOutcome,
  LauncherError,
  LauncherEventMap,
  PreferencesDto,
  PreferencesUpdate,
  SearchResponse,
  ShortcutStatus,
} from './model';

export type BridgeError = LauncherError | { code: string; message: string; recoverable: boolean };

type EventHandlers = {
  [Name in keyof LauncherEventMap]: (payload: LauncherEventMap[Name]) => void;
};

export function errorMessage(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string') {
    return error.message;
  }
  return String(error);
}

export async function signalLauncherReady(): Promise<void> {
  await invoke('launcher_ready');
}

export async function frameReady(generation: number, phase: 'activation' | 'results', rendererElapsedMs: number): Promise<void> {
  await invoke('frame_ready', { generation, phase, rendererElapsedMs });
}

export async function search(query: string, generation: number): Promise<SearchResponse> {
  return invoke<SearchResponse>('search', { query, generation });
}

export async function executeAction(resultId: string, actionId: string, generation: number): Promise<ActionOutcome> {
  return invoke<ActionOutcome>('execute_action', { resultId, actionId, generation });
}

export async function dismiss(): Promise<void> {
  await invoke('dismiss');
}

export async function getPreferences(): Promise<PreferencesDto> {
  return invoke<PreferencesDto>('get_preferences');
}

export async function savePreferences(request: PreferencesUpdate): Promise<PreferencesDto> {
  return invoke<PreferencesDto>('save_preferences', { request });
}

export async function configureShortcut(): Promise<ShortcutStatus> {
  return invoke<ShortcutStatus>('configure_shortcut');
}

export async function listenLauncherEvents(handlers: EventHandlers): Promise<() => void> {
  let disposed = false;
  const pending: Promise<UnlistenFn>[] = [];
  for (const name of Object.keys(handlers) as Array<keyof LauncherEventMap>) {
    const callback = handlers[name] as unknown as (payload: unknown) => void;
    const registration = listen<LauncherEventMap[typeof name]>(name, (event) => {
      if (!disposed) callback(event.payload);
    });
    pending.push(registration);
  }
  const registrations = await Promise.allSettled(pending);
  const unlisteners = registrations.flatMap((result) => result.status === 'fulfilled' ? [result.value] : []);
  const failed = registrations.find((result) => result.status === 'rejected');
  if (failed?.status === 'rejected') {
    disposed = true;
    for (const unlisten of unlisteners) unlisten();
    throw failed.reason;
  }
  return () => {
    disposed = true;
    for (const unlisten of unlisteners) unlisten();
  };
}
