use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::SearchSetupController;

impl SearchSetupController {
    pub(super) async fn enable_in_current_route(&self, root: PathBuf) -> Result<(), String> {
        let _transition = self.route_transition.lock().await;
        if self.reject_folder_search_on_selected_source() {
            return Err(
                "Folder search is paused while the selected Markdown source owns search."
                    .to_string(),
            );
        }
        match self.backend.enable(&self.state, root).await {
            Ok(()) => {
                *crate::application::lock(&self.selected) = None;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn resume_folder_route_if_current(&self) -> Result<(), String> {
        let _transition = self.route_transition.lock().await;
        if self.source_route_exclusive.load(Ordering::Acquire)
            || self.frontend.source_choice.is_selected()
        {
            return Ok(());
        }
        self.backend.resume(&self.state).await
    }

    pub(in crate::application) async fn suspend_for_source_choice(
        self: &Arc<Self>,
    ) -> Result<(), String> {
        self.source_route_exclusive.store(true, Ordering::Release);
        let _transition = self.route_transition.lock().await;
        if self.folder_suspended.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = self.backend.suspend().await;
        if result.is_ok() {
            self.folder_suspended.store(true, Ordering::Release);
        }
        self.publish(false);
        result
    }

    pub(in crate::application) async fn resume_after_source_removal(
        self: &Arc<Self>,
    ) -> Result<(), String> {
        let transition = self.route_transition.lock().await;
        if self.frontend.source_choice.is_selected() {
            return Err("The selected Markdown source still owns the search route.".to_string());
        }
        match self.backend.resume(&self.state).await {
            Ok(()) => {
                self.folder_suspended.store(false, Ordering::Release);
                self.source_route_exclusive.store(false, Ordering::Release);
                drop(transition);
                self.publish(true);
                Ok(())
            }
            Err(error) => match self.backend.suspend().await {
                Ok(()) => {
                    self.folder_suspended.store(true, Ordering::Release);
                    self.source_route_exclusive.store(false, Ordering::Release);
                    drop(transition);
                    self.publish(false);
                    Err(error)
                }
                Err(settlement) => {
                    self.folder_suspended.store(false, Ordering::Release);
                    self.source_route_exclusive.store(false, Ordering::Release);
                    drop(transition);
                    self.publish(false);
                    Err(format!(
                        "{error}; folder-search settlement also failed: {settlement}"
                    ))
                }
            },
        }
    }

    pub(super) fn reject_folder_search_on_selected_source(&self) -> bool {
        if self.source_route_exclusive.load(Ordering::Acquire)
            || self.frontend.source_choice.is_selected()
        {
            super::super::window::show_notice(
                &self.ui,
                "Folder search is paused while the selected Markdown source owns search."
                    .to_string(),
            );
            true
        } else {
            false
        }
    }
}
