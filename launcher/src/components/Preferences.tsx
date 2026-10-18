import { useEffect, useRef, useState } from 'react';
import type { PreferencesDto, PreferencesUpdate } from '../model';

type Props = {
  preferences: PreferencesDto | null;
  loading: boolean;
  busy: boolean;
  error: string | null;
  notice: string | null;
  resetConfirmation: boolean;
  onBack: () => void;
  onSave: (request: PreferencesUpdate) => void;
  onConfigure: () => void;
  onCopyActivation: () => void;
  onReset: () => void;
  onResetConfirmation: (value: boolean) => void;
};


export function Preferences({
  preferences,
  loading,
  error,
  notice,
  busy,
  resetConfirmation,
  onBack,
  onSave,
  onConfigure,
  onCopyActivation,
  onReset,
  onResetConfirmation,
}: Props) {
  const [shortcut, setShortcut] = useState(preferences?.shortcut ?? 'Control+Space');
  const [reduceMotion, setReduceMotion] = useState(preferences?.reduceMotion ?? false);
  const appliedPreferences = useRef<string | null>(null);
  const backButton = useRef<HTMLButtonElement>(null);
  const blocked = loading || busy || !preferences || preferences.readOnly;
  const applicationShortcut = preferences?.shortcutStatus.control === 'application';
  const shortcutDescription = preferences?.shortcutStatus.description ?? 'Checking native shortcut status';
  const configureLabel = {
    setup: 'Set Up Shortcut', change: 'Change Shortcut', rebind: 'Rebind Shortcut', retry: 'Retry Shortcut',
  }[preferences?.shortcutStatus.configureAction ?? 'setup'];
  useEffect(() => { backButton.current?.focus(); }, [loading]);

  useEffect(() => {
    if (!preferences) return;
    const key = `${preferences.shortcut}:${preferences.reduceMotion}:${preferences.shortcutSetup}`;
    if (appliedPreferences.current === key) return;
    appliedPreferences.current = key;
    setShortcut(preferences.shortcut);
    setReduceMotion(preferences.reduceMotion);
  }, [preferences]);


  function configure() {
    if (applicationShortcut && shortcut !== preferences?.shortcut) onSave({ shortcut });
    else onConfigure();
  }


  function confirmReset() {
    onReset();
    onResetConfirmation(false);
  }

  if (loading && !preferences) {
    return <div className="preferences-view" role="status" aria-live="polite">Loading preferences…</div>;
  }

  return (
    <section className="preferences-view" aria-labelledby="preferences-title">
      <div className="preferences-heading">
        <button ref={backButton} type="button" className="back-button" onClick={onBack} aria-label="Back to results">Back</button>
        <div>
          <h1 id="preferences-title">Preferences</h1>
          <p>Launcher behavior and keyboard activation</p>
        </div>
      </div>
      {error ? <p className="preference-error" role="alert">{error}</p> : null}
      {!error && notice ? <p className="preference-notice" role="status">{notice}</p> : null}
      {preferences?.warning ? <p className="preference-warning" role="status">{preferences.warning}</p> : null}
      <div className="preference-grid">
        <label className="preference-field">
          <span>Shortcut</span>
          {applicationShortcut ? (
            <input type="text" value={shortcut} disabled={blocked}
              onChange={(event) => setShortcut(event.currentTarget.value)} />
          ) : (
            <output>{shortcutDescription}</output>
          )}
          {applicationShortcut && preferences ? <small>{shortcutDescription}</small> : null}
          {preferences?.shortcutStatus.control === 'system' ? <small>Managed by your desktop’s shortcut dialog.</small> : null}
          {preferences?.shortcutStatus.message ? <small role="status">{preferences.shortcutStatus.message}</small> : null}
        </label>
        <label className="preference-switch">
          <input
            type="checkbox"
            checked={reduceMotion}
            disabled={blocked}
            onChange={(event) => setReduceMotion(event.currentTarget.checked)}
          />
          <span>Reduce motion</span>
        </label>
      </div>
      <div className="shortcut-actions">
        <button type="button" className="primary-button" disabled={blocked || preferences?.shortcutStatus.control === 'unavailable'} onClick={configure}>{configureLabel}</button>
        {preferences?.shortcutSetup === 'unconfigured' ? <button type="button" className="secondary-button" disabled={blocked} onClick={() => onSave({ shortcutSetup: 'deferred' })}>Not Now</button> : null}
      </div>
      <div className="preference-actions">
        <button type="button" className="secondary-button" disabled={loading || busy} onClick={onCopyActivation}>Copy activation command</button>
        <button type="button" className="secondary-button" disabled={blocked} onClick={() => onResetConfirmation(true)}>Reset preferences</button>
        <button type="button" className="primary-button" disabled={blocked}
          onClick={() => onSave({ reduceMotion, ...(applicationShortcut && shortcut !== preferences?.shortcut ? { shortcut } : {}) })}>Save</button>
      </div>
      {resetConfirmation ? (
        <div className="reset-confirmation" role="alertdialog" aria-label="Confirm reset">
          <p>Reset saved preferences to their defaults?</p>
          <div>
            <button type="button" className="secondary-button" onClick={() => onResetConfirmation(false)}>Cancel</button>
            <button type="button" className="danger-button" disabled={blocked} onClick={confirmReset}>Confirm reset</button>
          </div>
        </div>
      ) : null}
      <p className="preference-platform">Platform: {preferences?.platform ?? 'unknown'}</p>
    </section>
  );
}
