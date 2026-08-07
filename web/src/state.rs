use std::collections::BTreeMap;

use crate::api::{
    Agent, AskContext, CatalogSource, ClientError, DraftPreview, DraftSummary, Notebook,
    NotebookSummary,
};

pub const MAX_HISTORY_MESSAGES: usize = 12;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum LoadState<T> {
    #[default]
    Loading,
    Ready(T),
    Empty,
    Failed(ClientError),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestEpochs {
    epochs: BTreeMap<u64, u64>,
}
impl RequestEpochs {
    pub fn begin(&mut self, notebook_id: u64) -> (u64, u64) {
        let epoch = self
            .epochs
            .entry(notebook_id)
            .and_modify(|value| *value += 1)
            .or_insert(1);
        (notebook_id, *epoch)
    }
    pub fn is_current(&self, request: (u64, u64), active_notebook: u64) -> bool {
        request.0 == active_notebook && self.epochs.get(&request.0).copied() == Some(request.1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryMessage {
    pub role: String,
    pub markdown: String,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AskHistory {
    messages: Vec<HistoryMessage>,
}
impl AskHistory {
    pub fn messages(&self) -> &[HistoryMessage] {
        &self.messages
    }
    pub fn push_pair(&mut self, question: impl Into<String>, answer: impl Into<String>) {
        self.messages.push(HistoryMessage {
            role: "user".into(),
            markdown: question.into(),
        });
        self.messages.push(HistoryMessage {
            role: "assistant".into(),
            markdown: answer.into(),
        });
        if self.messages.len() > MAX_HISTORY_MESSAGES {
            let excess = self.messages.len() - MAX_HISTORY_MESSAGES;
            self.messages.drain(..excess);
        }
    }
    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewState {
    pub title: String,
    pub markdown: String,
    pub evidence_ids: Vec<u64>,
    pub draft_id: Option<u64>,
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StudioStateModel {
    pub notebooks: LoadState<Vec<NotebookSummary>>,
    pub agents: Vec<Agent>,
    pub notebook: LoadState<Notebook>,
    pub sources: LoadState<Vec<CatalogSource>>,
    pub drafts: LoadState<Vec<DraftSummary>>,
    pub ask_history: AskHistory,
    pub ask_notebook: Option<u64>,
    pub ask_epoch: u64,
    pub answer: Option<String>,
    pub context: Option<AskContext>,
    pub preview: Option<PreviewState>,
    pub draft_previews: Vec<DraftPreview>,
    pub alert: Option<ClientError>,
    pub status: String,
    pub epochs: RequestEpochs,
}
impl StudioStateModel {
    fn invalidate_ask(&mut self) {
        self.ask_epoch = if self.ask_epoch == u64::MAX {
            0
        } else {
            self.ask_epoch + 1
        };
    }

    pub fn begin_notebook_request(&mut self, id: u64) -> (u64, u64) {
        if self.ask_notebook != Some(id) {
            self.invalidate_ask();
            self.ask_history.clear();
            self.answer = None;
            self.context = None;
            self.preview = None;
            self.draft_previews.clear();
            self.ask_notebook = Some(id);
        }
        self.epochs.begin(id)
    }

    pub fn begin_ask(&mut self, id: u64) -> (u64, u64) {
        self.invalidate_ask();
        (id, self.ask_epoch)
    }

    pub fn is_current_ask(&self, request: (u64, u64), active: u64) -> bool {
        request.0 == active && self.ask_notebook == Some(active) && self.ask_epoch == request.1
    }

    pub fn clear_ask(&mut self, id: u64, active: u64) {
        if id == active {
            self.invalidate_ask();
            self.ask_history.clear();
            self.answer = None;
            self.context = None;
            self.preview = None;
            self.draft_previews.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_notebook_epoch_cannot_commit() {
        let mut epochs = RequestEpochs::default();
        let first = epochs.begin(1);
        let _second = epochs.begin(1);
        assert!(!epochs.is_current(first, 1));
    }
    #[test]
    fn different_notebook_cannot_commit() {
        let mut epochs = RequestEpochs::default();
        let request = epochs.begin(1);
        assert!(!epochs.is_current(request, 2));
    }
    #[test]
    fn changing_notebooks_clears_transient_ask_state() {
        let mut model = StudioStateModel {
            ask_notebook: Some(1),
            answer: Some("old answer".into()),
            ..StudioStateModel::default()
        };
        model.ask_history.push_pair("old question", "old answer");
        model.begin_notebook_request(2);
        assert_eq!(model.ask_notebook, Some(2));
        assert!(model.ask_history.messages().is_empty());
        assert!(model.answer.is_none());
        assert!(model.context.is_none());
        assert!(model.preview.is_none());
        assert!(model.draft_previews.is_empty());
    }
    #[test]
    fn stale_ask_epoch_cannot_commit_after_refresh_or_clear() {
        let mut model = StudioStateModel {
            ask_notebook: Some(1),
            ..StudioStateModel::default()
        };
        let first = model.begin_ask(1);
        let second = model.begin_ask(1);
        assert!(!model.is_current_ask(first, 1));
        assert!(model.is_current_ask(second, 1));
        model.clear_ask(1, 1);
        assert!(!model.is_current_ask(second, 1));
    }
    #[test]
    fn history_is_bounded_to_six_pairs() {
        let mut history = AskHistory::default();
        for index in 0..8 {
            history.push_pair(format!("q{index}"), format!("a{index}"));
        }
        assert_eq!(history.messages().len(), 12);
        assert_eq!(history.messages()[0].markdown, "q2");
    }
}
