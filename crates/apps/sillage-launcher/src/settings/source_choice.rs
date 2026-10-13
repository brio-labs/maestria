use sillage_domain::{ControlPersistenceChoice, DocumentaryRetentionChoice, SourceScope};

use super::SettingsManager;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceControlPreferences {
    pub(crate) documentary_retention: DocumentaryRetentionChoice,
    pub(crate) control_persistence: ControlPersistenceChoice,
    pub(crate) selected_scope: Option<SourceScope>,
}

impl Default for SourceControlPreferences {
    fn default() -> Self {
        Self {
            documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
            control_persistence: ControlPersistenceChoice::SessionOnly,
            selected_scope: None,
        }
    }
}

impl SettingsManager {
    pub(crate) fn source_control_preferences(&self) -> SourceControlPreferences {
        let documentary_retention = match self.values.source_documentary_retention {
            Some(choice) => choice,
            None => DocumentaryRetentionChoice::NoDocumentaryPersistence,
        };
        let control_persistence = match self.values.source_control_persistence {
            Some(choice) => choice,
            None => ControlPersistenceChoice::SessionOnly,
        };
        let selected_scope = (control_persistence == ControlPersistenceChoice::PersistExplicitly)
            .then(|| self.values.source_scope.clone())
            .flatten();
        SourceControlPreferences {
            documentary_retention,
            control_persistence,
            selected_scope,
        }
    }

    pub(crate) fn set_source_control_preferences(
        &mut self,
        documentary_retention: DocumentaryRetentionChoice,
        control_persistence: ControlPersistenceChoice,
        selected_scope: Option<SourceScope>,
    ) -> Result<(), String> {
        if self.read_only {
            return Err("Preferences from an unknown schema version are read-only".to_string());
        }
        let persist_controls = control_persistence == ControlPersistenceChoice::PersistExplicitly;
        if persist_controls && (self.preserve_existing || self.path.is_none()) {
            return Err(if self.preserve_existing {
                "The existing preferences file was preserved; confirm Reset before remembering source controls".to_string()
            } else {
                "Remembering source controls requires a persistent preferences directory"
                    .to_string()
            });
        }
        let retained_scope = persist_controls.then_some(selected_scope).flatten();
        let next_documentary = persist_controls.then_some(documentary_retention);
        let next_control = persist_controls.then_some(control_persistence);
        if self.values.source_documentary_retention == next_documentary
            && self.values.source_control_persistence == next_control
            && self.values.source_scope == retained_scope
        {
            return Ok(());
        }
        let previous = (
            self.values.source_documentary_retention,
            self.values.source_control_persistence,
            self.values.source_scope.clone(),
        );
        self.values.source_documentary_retention = next_documentary;
        self.values.source_control_persistence = next_control;
        self.values.source_scope = retained_scope;
        if let Err(error) = self.persist() {
            self.values.source_documentary_retention = previous.0;
            self.values.source_control_persistence = previous.1;
            self.values.source_scope = previous.2;
            return Err(error);
        }
        Ok(())
    }
}

pub(super) fn validate_source_control_settings(
    settings: &super::PersistedSettings,
) -> Result<(), String> {
    let has_documentary = settings.source_documentary_retention.is_some();
    let has_control_choice = settings.source_control_persistence.is_some();
    if has_documentary != has_control_choice
        || settings.source_control_persistence == Some(ControlPersistenceChoice::SessionOnly)
        || (settings.source_scope.is_some()
            && settings.source_control_persistence
                != Some(ControlPersistenceChoice::PersistExplicitly))
        || (settings.source_scope.is_some()
            && !settings
                .source_scope
                .as_ref()
                .is_some_and(|scope| scope.path().is_absolute()))
    {
        return Err(
            "Remembered source controls are inconsistent or contain a selected path without explicit control persistence"
                .to_string(),
        );
    }
    Ok(())
}
