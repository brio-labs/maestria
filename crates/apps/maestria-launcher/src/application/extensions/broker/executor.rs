use maestria_extensions::{
    CapabilityError, CapabilityFailure, CapabilityRequest, CapabilityResponse, CapabilitySuccess,
    GrantError, HttpMethod, InvocationOrigin, OpenRequestTarget, Permission, StorageOperation,
    authorize,
};

use super::{
    CODE_FAILED, CODE_INVALID_REQUEST, CODE_NOT_FOUND, CODE_PERMISSION_DENIED, CODE_UNAVAILABLE,
    CapabilityBroker, desktop, network, search, storage,
};

impl CapabilityBroker {
    /// `active_grants` must be freshly loaded for this extension and request.
    /// `origin` must come from a trusted host callback, never worker input.
    /// # Cancellation
    ///
    /// Dropping this future drops any in-flight search, HTTP, or selected-file-open future
    /// instead of awaiting its response. It cannot undo synchronous storage, notification,
    /// clipboard, or URI-launch effects already performed, or guarantee reversal of an
    /// operation an external service has accepted.
    ///
    /// Worker termination is owned by the invocation task, not this method. In the launcher,
    /// aborting that task drops its `WorkerSession`, whose child uses `kill_on_drop(true)`.
    pub async fn execute(
        &self,
        active_grants: &[Permission],
        request: &CapabilityRequest,
        origin: InvocationOrigin,
    ) -> CapabilityResponse {
        if let Err(error) = authorize(active_grants, request, origin) {
            return authorization_failure(request, error);
        }
        match request {
            CapabilityRequest::FileSearch { query, limit } => {
                self.file_search(request, query, *limit).await
            }
            CapabilityRequest::UserFileRead {
                selection_id,
                max_bytes,
            } => self.user_file_read(request, selection_id, *max_bytes),
            CapabilityRequest::Http { url, method, body } => {
                self.http(request, url, *method, body.as_deref()).await
            }
            CapabilityRequest::Storage {
                operation,
                key,
                value,
            } => self.storage(request, *operation, key, value.as_deref()),
            CapabilityRequest::Notification { title, message } => {
                self.notification(request, title, message)
            }
            CapabilityRequest::Open { target } => self.open(request, target).await,
            CapabilityRequest::Copy { text } => self.copy(request, text),
        }
    }

    async fn file_search(
        &self,
        request: &CapabilityRequest,
        query: &str,
        limit: usize,
    ) -> CapabilityResponse {
        let Some(config) = self.context.search_consumer.as_ref() else {
            return failure(
                request,
                CODE_UNAVAILABLE,
                "The extension search service is not configured.",
            );
        };
        match search::file_search(config, query, limit).await {
            Ok(results) => {
                CapabilityResponse::Success(CapabilitySuccess::FileSearch { ok: true, results })
            }
            Err(search::SearchError::Unavailable) => failure(
                request,
                CODE_UNAVAILABLE,
                "The extension search service is unavailable.",
            ),
            Err(search::SearchError::InvalidRequest) => failure(
                request,
                CODE_INVALID_REQUEST,
                "The file search query is not supported.",
            ),
            Err(search::SearchError::Failed) => {
                failure(request, CODE_FAILED, "The extension search request failed.")
            }
        }
    }

    fn user_file_read(
        &self,
        request: &CapabilityRequest,
        selection_id: &str,
        max_bytes: usize,
    ) -> CapabilityResponse {
        let Some(file) = self.context.selected_files.get(selection_id) else {
            return failure(request, CODE_NOT_FOUND, "The selected file is unavailable.");
        };
        match file.read_text(max_bytes) {
            Ok((text, truncated)) => CapabilityResponse::Success(CapabilitySuccess::UserFileRead {
                ok: true,
                text,
                truncated,
            }),
            Err(error) => failure(
                request,
                io_failure_code(&error),
                "The selected file could not be read safely.",
            ),
        }
    }

    async fn http(
        &self,
        request: &CapabilityRequest,
        url: &str,
        method: HttpMethod,
        body: Option<&str>,
    ) -> CapabilityResponse {
        match network::request(url, method, body).await {
            Ok(response) => CapabilityResponse::Success(CapabilitySuccess::Http {
                ok: true,
                status: response.status,
                body: response.body,
                truncated: response.truncated,
            }),
            Err(network::NetworkError::Denied) => failure(
                request,
                CODE_PERMISSION_DENIED,
                "The HTTP destination is not permitted.",
            ),
            Err(network::NetworkError::InvalidRequest) => failure(
                request,
                CODE_INVALID_REQUEST,
                "The HTTP request exceeds the supported bounds.",
            ),
            Err(network::NetworkError::Unavailable) => failure(
                request,
                CODE_UNAVAILABLE,
                "The HTTPS service is unavailable.",
            ),
            Err(network::NetworkError::Failed) => failure(
                request,
                CODE_FAILED,
                "The HTTPS response could not be processed.",
            ),
        }
    }

    fn storage(
        &self,
        request: &CapabilityRequest,
        operation: StorageOperation,
        key: &str,
        value: Option<&str>,
    ) -> CapabilityResponse {
        match storage::execute(
            &self.context.storage_root,
            &self.context.extension_id,
            operation,
            key,
            value,
        ) {
            Ok(value) => CapabilityResponse::Success(value),
            Err(storage::StorageError::NotFound) => failure(
                request,
                CODE_NOT_FOUND,
                "The extension storage key was not found.",
            ),
            Err(storage::StorageError::InvalidRequest) => failure(
                request,
                CODE_INVALID_REQUEST,
                "The storage value or extension quota exceeds its bound.",
            ),
            Err(storage::StorageError::Unavailable) => failure(
                request,
                CODE_UNAVAILABLE,
                "Extension storage is unavailable.",
            ),
            Err(storage::StorageError::Failed) => {
                failure(request, CODE_FAILED, "The storage operation failed.")
            }
        }
    }

    fn notification(
        &self,
        request: &CapabilityRequest,
        title: &str,
        message: &str,
    ) -> CapabilityResponse {
        match desktop::notify(title, message) {
            Ok(()) => CapabilityResponse::Success(CapabilitySuccess::Notification {
                ok: true,
                delivered: true,
            }),
            Err(desktop::DesktopError::Unavailable) => failure(
                request,
                CODE_UNAVAILABLE,
                "Desktop notifications are unavailable.",
            ),
            Err(desktop::DesktopError::NotFound | desktop::DesktopError::Failed) => failure(
                request,
                CODE_FAILED,
                "The desktop notification could not be delivered.",
            ),
        }
    }

    async fn open(
        &self,
        request: &CapabilityRequest,
        target: &OpenRequestTarget,
    ) -> CapabilityResponse {
        let result = match target {
            OpenRequestTarget::Url { url } => desktop::open_url(url),
            OpenRequestTarget::SelectedFile { selection_id } => {
                self.open_selected_file(selection_id).await
            }
        };
        match result {
            Ok(()) => CapabilityResponse::Success(CapabilitySuccess::Open {
                ok: true,
                opened: true,
            }),
            Err(desktop::DesktopError::Unavailable) => failure(
                request,
                CODE_UNAVAILABLE,
                "The desktop open service is unavailable.",
            ),
            Err(desktop::DesktopError::NotFound) => {
                failure(request, CODE_NOT_FOUND, "The selected file is unavailable.")
            }
            Err(desktop::DesktopError::Failed) => failure(
                request,
                CODE_FAILED,
                "The requested target could not be opened.",
            ),
        }
    }

    async fn open_selected_file(&self, selection_id: &str) -> Result<(), desktop::DesktopError> {
        let file = self
            .context
            .selected_files
            .get(selection_id)
            .ok_or(desktop::DesktopError::NotFound)?;
        desktop::open_selected_file(file.descriptor()).await
    }

    fn copy(&self, request: &CapabilityRequest, text: &str) -> CapabilityResponse {
        match desktop::copy_text(text) {
            Ok(()) => CapabilityResponse::Success(CapabilitySuccess::Copy {
                ok: true,
                copied: true,
            }),
            Err(desktop::DesktopError::Unavailable) => {
                failure(request, CODE_UNAVAILABLE, "The clipboard is unavailable.")
            }
            Err(desktop::DesktopError::NotFound | desktop::DesktopError::Failed) => {
                failure(request, CODE_FAILED, "Clipboard text could not be copied.")
            }
        }
    }
}

fn authorization_failure(request: &CapabilityRequest, error: GrantError) -> CapabilityResponse {
    match error {
        GrantError::Invalid(_) => failure(
            request,
            CODE_INVALID_REQUEST,
            "The capability request is invalid.",
        ),
        GrantError::Missing(_) | GrantError::Scope(_) | GrantError::UserAction(_) => failure(
            request,
            CODE_PERMISSION_DENIED,
            "The active extension grant does not authorize this request.",
        ),
    }
}

fn failure(request: &CapabilityRequest, code: &str, message: &str) -> CapabilityResponse {
    CapabilityResponse::Failure(CapabilityFailure {
        ok: false,
        capability: request.kind().to_owned(),
        error: CapabilityError {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    })
}

fn io_failure_code(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => CODE_NOT_FOUND,
        std::io::ErrorKind::InvalidInput => CODE_INVALID_REQUEST,
        std::io::ErrorKind::Unsupported => CODE_UNAVAILABLE,
        _ => CODE_FAILED,
    }
}
