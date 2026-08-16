use gloo_net::http::{Request, RequestBuilder};
use serde::{Serialize, de::DeserializeOwned};

pub use crate::api_types::*;
pub use crate::index_types::*;
pub use crate::repository_index_types::*;
use crate::session::Session;

#[derive(Clone, Debug, PartialEq)]
pub struct ApiClient {
    session: Session,
}
#[path = "api/support.rs"]
mod support;

use support::{browse_input, encode_source_key};

impl ApiClient {
    pub fn new() -> Self {
        Self {
            session: Session::from_browser(),
        }
    }
    pub fn with_session(session: Session) -> Self {
        Self { session }
    }
    fn builder(&self, method: &str, path: &str) -> Result<RequestBuilder, ClientError> {
        let builder = match method {
            "GET" => Request::get(path),
            "POST" => Request::post(path),
            "PUT" => Request::put(path),
            "PATCH" => Request::patch(path),
            "DELETE" => Request::delete(path),
            _ => {
                return Err(ClientError::InvalidResponse(
                    "unsupported HTTP method".into(),
                ));
            }
        };
        Ok(builder
            .header(
                "Authorization",
                &format!("Bearer {}", self.session.bearer()),
            )
            .header("Content-Type", "application/json"))
    }
    fn empty(&self, method: &str, path: &str) -> Result<Request, ClientError> {
        let builder = self.builder(method, path)?;
        if matches!(method, "GET" | "HEAD") {
            builder
                .build()
                .map_err(|error| ClientError::InvalidResponse(error.to_string()))
        } else {
            builder
                .body("")
                .map_err(|error| ClientError::InvalidResponse(error.to_string()))
        }
    }
    fn json<T: Serialize>(
        &self,
        method: &str,
        path: &str,
        value: &T,
    ) -> Result<Request, ClientError> {
        self.builder(method, path)?
            .json(value)
            .map_err(|error| ClientError::InvalidResponse(error.to_string()))
    }
    async fn send<T: DeserializeOwned>(&self, request: Request) -> Result<T, ClientError> {
        let response = request
            .send()
            .await
            .map_err(|error| ClientError::Network(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .binary()
            .await
            .map_err(|error| ClientError::Network(error.to_string()))?;
        if !(200..300).contains(&status) {
            let problem = serde_json::from_slice::<ProblemDetails>(&bytes)
                .map_err(|error| ClientError::InvalidResponse(format!("HTTP {status}: {error}")))?;
            return Err(ClientError::Problem(problem));
        }
        serde_json::from_slice(&bytes)
            .or_else(|_| {
                serde_json::from_slice::<Envelope<T>>(&bytes).map(|envelope| envelope.data)
            })
            .map_err(|error| ClientError::InvalidResponse(error.to_string()))
    }
    async fn send_status(&self, request: Request) -> Result<(), ClientError> {
        let response = request
            .send()
            .await
            .map_err(|error| ClientError::Network(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .binary()
            .await
            .map_err(|error| ClientError::Network(error.to_string()))?;
        if !(200..300).contains(&status) {
            let problem = serde_json::from_slice::<ProblemDetails>(&bytes)
                .map_err(|error| ClientError::InvalidResponse(format!("HTTP {status}: {error}")))?;
            return Err(ClientError::Problem(problem));
        }
        Ok(())
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn bootstrap(&self) -> Result<Bootstrap, ClientError> {
        self.send(self.empty("GET", "/api/bootstrap")?).await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn notebooks(&self) -> Result<Vec<NotebookSummary>, ClientError> {
        let response: Envelope<NotebookListPayload> =
            self.send(self.empty("GET", "/api/notebooks")?).await?;
        Ok(response.data.notebooks)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn notebook(&self, id: u64) -> Result<Notebook, ClientError> {
        self.send(self.empty("GET", &format!("/api/notebooks/{id}"))?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn create_notebook(&self, title: String) -> Result<NotebookSummary, ClientError> {
        self.send(self.json(
            "POST",
            "/api/notebooks",
            &serde_json::json!({"title": title}),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn rename_notebook(&self, id: u64, title: String) -> Result<Notebook, ClientError> {
        self.send(self.json(
            "PATCH",
            &format!("/api/notebooks/{id}"),
            &serde_json::json!({"title": title}),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn delete_notebook(&self, id: u64) -> Result<(), ClientError> {
        self.send_status(self.empty("DELETE", &format!("/api/notebooks/{id}"))?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn sources(&self, id: u64) -> Result<Vec<CatalogSource>, ClientError> {
        let response: Envelope<SourceCatalogWire> = self
            .send(self.empty("GET", &format!("/api/notebooks/{id}/sources"))?)
            .await?;
        Ok(response.data.sources)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn attach_source(&self, id: u64, key: &str) -> Result<(), ClientError> {
        self.send_status(self.empty(
            "POST",
            &format!("/api/notebooks/{id}/sources/{}", encode_source_key(key)),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn detach_source(&self, id: u64, key: &str) -> Result<(), ClientError> {
        self.send_status(self.empty(
            "DELETE",
            &format!("/api/notebooks/{id}/sources/{}", encode_source_key(key)),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn drafts(&self, id: u64) -> Result<Vec<DraftSummary>, ClientError> {
        let response: Envelope<DraftListWire> = self
            .send(self.empty("GET", &format!("/api/notebooks/{id}/drafts"))?)
            .await?;
        Ok(response.data.drafts)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn draft(&self, notebook_id: u64, draft_id: u64) -> Result<Draft, ClientError> {
        self.send(self.empty(
            "GET",
            &format!("/api/notebooks/{notebook_id}/drafts/{draft_id}"),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn create_draft(
        &self,
        id: u64,
        input: &CreateDraft,
    ) -> Result<SavedDraft, ClientError> {
        self.send(self.json("POST", &format!("/api/notebooks/{id}/drafts"), input)?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn update_draft(
        &self,
        notebook_id: u64,
        draft_id: u64,
        input: &UpdateDraft,
    ) -> Result<SavedDraft, ClientError> {
        self.send(self.json(
            "PATCH",
            &format!("/api/notebooks/{notebook_id}/drafts/{draft_id}"),
            input,
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn delete_draft(
        &self,
        notebook_id: u64,
        draft_id: u64,
        revision: u64,
    ) -> Result<(), ClientError> {
        self.send_status(self.json(
            "DELETE",
            &format!("/api/notebooks/{notebook_id}/drafts/{draft_id}"),
            &DeleteDraft {
                expected_revision: revision,
            },
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn ask(
        &self,
        notebook_id: u64,
        input: &AskRequest,
    ) -> Result<AskResponse, ClientError> {
        self.send(self.json("POST", &format!("/api/notebooks/{notebook_id}/ask"), input)?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn evidence(
        &self,
        notebook_id: u64,
        evidence_id: u64,
    ) -> Result<Evidence, ClientError> {
        self.send(self.empty(
            "GET",
            &format!("/api/notebooks/{notebook_id}/evidence/{evidence_id}"),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn search(&self, query: &str, limit: usize) -> Result<SearchResponse, ClientError> {
        self.send(self.empty(
            "GET",
            &format!(
                "/api/search?query={}&limit={limit}",
                encode_source_key(query)
            ),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn retrieval(&self) -> Result<RetrievalStatus, ClientError> {
        self.send(self.empty("GET", "/api/retrieval")?).await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn tasks(&self) -> Result<Vec<TaskSummaryWire>, ClientError> {
        let response: Envelope<TaskListWire> = self.send(self.empty("GET", "/api/tasks")?).await?;
        Ok(response.data.tasks)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn evidence_global(&self, evidence_id: u64) -> Result<Evidence, ClientError> {
        self.send(self.empty("GET", &format!("/api/evidence/{evidence_id}"))?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn index_candidates(&self, root: &str) -> Result<IndexCandidatesWire, ClientError> {
        self.send(self.empty(
            "GET",
            &format!("/api/index/candidates/{}", encode_source_key(root)),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn index_selection(&self) -> Result<Option<IndexSelectionProfileWire>, ClientError> {
        let response: Envelope<IndexSelectionResponseWire> = self
            .send(self.empty("GET", "/api/index/selection")?)
            .await?;
        Ok(response.data.profile)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn index_selection_save(
        &self,
        profile: &IndexSelectionProfileWire,
    ) -> Result<(), ClientError> {
        self.send_status(self.json("PUT", "/api/index/selection", profile)?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn index_run(
        &self,
        root: &str,
        includes: Vec<String>,
        policies: std::collections::BTreeMap<String, IndexPolicyWire>,
    ) -> Result<IndexRunWire, ClientError> {
        self.send(self.json(
            "POST",
            "/api/index/run",
            &IndexRunInputWire {
                root: root.to_string(),
                includes,
                policies,
            },
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_candidates(
        &self,
        root: &str,
    ) -> Result<RepositoryIndexCandidatesWire, ClientError> {
        self.send(self.empty(
            "GET",
            &format!(
                "/api/repository-index/candidates/{}",
                encode_source_key(root)
            ),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_selection(
        &self,
    ) -> Result<Option<IndexSelectionProfileWire>, ClientError> {
        let response: Envelope<RepositoryIndexSelectionResponseWire> = self
            .send(self.empty("GET", "/api/repository-index/selection")?)
            .await?;
        Ok(response.data.profile)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_selection_save(
        &self,
        profile: &IndexSelectionProfileWire,
    ) -> Result<(), ClientError> {
        self.send_status(self.json("PUT", "/api/repository-index/selection", profile)?)
            .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_run(
        &self,
        root: &str,
        includes: Vec<String>,
        policies: std::collections::BTreeMap<String, IndexPolicyWire>,
    ) -> Result<RepositoryIndexRunWire, ClientError> {
        self.send(self.json(
            "POST",
            "/api/repository-index/run",
            &RepositoryIndexRunInputWire {
                root: root.to_string(),
                includes,
                policies,
            },
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_status(
        &self,
        root: &str,
    ) -> Result<RepositoryIndexStatusWire, ClientError> {
        self.send(self.empty(
            "GET",
            &format!("/api/repository-index/status/{}", encode_source_key(root)),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_children(
        &self,
        root: &str,
        path: &str,
    ) -> Result<Vec<CandidateDirWire>, ClientError> {
        let response: RepositoryIndexChildrenWire = self
            .send(self.json(
                "POST",
                "/api/repository-index/children",
                &browse_input(root, path),
            )?)
            .await?;
        Ok(response.children)
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_files(
        &self,
        root: &str,
        path: &str,
    ) -> Result<RepositoryIndexFilesWire, ClientError> {
        self.send(self.json(
            "POST",
            "/api/repository-index/files",
            &browse_input(root, path),
        )?)
        .await
    }
    /// # Cancellation
    /// Dropping the future cancels the browser request.
    pub async fn repository_index_progress(
        &self,
    ) -> Result<Option<RepositoryIndexProgressWire>, ClientError> {
        let response: Envelope<RepositoryIndexProgressResponseWire> = self
            .send(self.empty("GET", "/api/repository-index/progress")?)
            .await?;
        Ok(response.data.progress)
    }
}
impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn problem_branch_uses_full_type_uri() {
        let error = ClientError::Problem(ProblemDetails {
            type_uri: "urn:maestria:studio:problem:revision-conflict".into(),
            title: "Revision conflict".into(),
            status: 409,
            detail: "reload".into(),
        });
        assert_eq!(error.problem_code(), Some("revision-conflict"));
    }
    #[test]
    fn bootstrap_decodes_typed_notebook_payload() -> Result<(), serde_json::Error> {
        let bootstrap: Bootstrap = serde_json::from_str(super::support::BOOTSTRAP_JSON)?;
        let notebooks = bootstrap.notebooks.into_vec();
        assert_eq!(notebooks.first().map(|item| item.notebook_id), Some(1));
        Ok(())
    }
}
