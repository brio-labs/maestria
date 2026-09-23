use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use tokio::sync::oneshot;

use crate::catalog::CatalogSnapshot;
use crate::errors::LauncherError;
use crate::model::{
    Action, ResultKind, SearchResponse, SearchResult, SearchStatus, SearchStatusKind,
};

const QUERY_QUEUE_CAPACITY: usize = 16;

struct PendingQuery {
    query: String,
    generation: u64,
    catalog: Arc<CatalogSnapshot>,
    response: oneshot::Sender<Result<SearchResponse, LauncherError>>,
}

struct QueryQueue {
    pending: VecDeque<PendingQuery>,
    shutdown: bool,
    latest_generation: u64,
}

/// A bounded worker queue that always evaluates the newest pending generation.
pub struct QueryWorker {
    queue: Arc<(Mutex<QueryQueue>, Condvar)>,
}

impl QueryWorker {
    pub fn new() -> Result<Self, LauncherError> {
        let queue = Arc::new((
            Mutex::new(QueryQueue {
                pending: VecDeque::with_capacity(QUERY_QUEUE_CAPACITY),
                shutdown: false,
                latest_generation: 0,
            }),
            Condvar::new(),
        ));
        let worker_queue = Arc::clone(&queue);
        std::thread::Builder::new()
            .name("maestria-launcher-query".to_string())
            .spawn(move || query_worker_loop(worker_queue))
            .map_err(|error| {
                LauncherError::platform_unavailable(format!(
                    "The launcher query worker could not start: {error}"
                ))
            })?;
        Ok(Self { queue })
    }

    /// Submit a generation-tagged query; older pending requests are completed as stale.
    ///
    /// # Cancellation
    ///
    /// Dropping the returned future after submission does not cancel worker evaluation.
    /// A later generation supersedes the request with `stale_result`; dropping the worker
    /// instead causes a `platform_unavailable` result for requests still awaiting it.
    pub async fn submit(
        &self,
        query: String,
        generation: u64,
        catalog: Arc<CatalogSnapshot>,
    ) -> Result<SearchResponse, LauncherError> {
        let (response_tx, response_rx) = oneshot::channel();
        {
            let (queue_lock, wake) = &*self.queue;
            let mut queue = queue_lock.lock().map_err(|_| {
                LauncherError::platform_unavailable("The launcher query worker is unavailable")
            })?;
            if queue.shutdown {
                return Err(LauncherError::platform_unavailable(
                    "The launcher query worker could not start",
                ));
            }
            if generation <= queue.latest_generation {
                return Err(LauncherError::stale_result(
                    "A newer query superseded this request",
                ));
            }
            queue.latest_generation = generation;
            while queue.pending.len() >= QUERY_QUEUE_CAPACITY {
                if let Some(oldest) = queue.pending.pop_front() {
                    let _ = oldest.response.send(Err(LauncherError::stale_result(
                        "A newer query superseded this request",
                    )));
                }
            }
            queue.pending.push_back(PendingQuery {
                query,
                generation,
                catalog,
                response: response_tx,
            });
            wake.notify_one();
        }
        response_rx.await.map_err(|_| {
            LauncherError::platform_unavailable("The launcher query worker stopped unexpectedly")
        })?
    }
}

impl Drop for QueryWorker {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.queue.0.lock() {
            queue.shutdown = true;
            self.queue.1.notify_one();
        }
    }
}

fn query_worker_loop(queue: Arc<(Mutex<QueryQueue>, Condvar)>) {
    loop {
        let request = {
            let (queue_lock, wake) = &*queue;
            let mut queue = match queue_lock.lock() {
                Ok(queue) => queue,
                Err(error) => error.into_inner(),
            };
            while queue.pending.is_empty() && !queue.shutdown {
                queue = match wake.wait(queue) {
                    Ok(queue) => queue,
                    Err(error) => error.into_inner(),
                };
            }
            if queue.shutdown {
                return;
            }
            let newest = queue.pending.pop_back();
            let superseded: Vec<PendingQuery> = queue.pending.drain(..).collect();
            drop(queue);
            for stale in superseded {
                let _ = stale.response.send(Err(LauncherError::stale_result(
                    "A newer query superseded this request",
                )));
            }
            newest
        };
        let Some(request) = request else {
            continue;
        };
        let mut results = crate::query::search_catalog(&request.query, &request.catalog.apps);
        let mut status = request.catalog.status.clone();
        match crate::calculator::calculate(&request.query) {
            Ok(Some(value)) => {
                results.truncate(49);
                results.insert(
                    0,
                    SearchResult {
                        id: "calculation".to_string(),
                        kind: ResultKind::Calculation,
                        title: value,
                        subtitle: request.query.clone(),
                        icon: None,
                        actions: vec![Action {
                            id: "copy-result".to_string(),
                            title: "Copy Result".to_string(),
                            primary: true,
                        }],
                    },
                );
            }
            Ok(None) => {}
            Err(error) => {
                status = SearchStatus {
                    kind: SearchStatusKind::CalculationError,
                    message: Some(error.to_string()),
                };
            }
        }
        let response = SearchResponse {
            generation: request.generation,
            catalog_revision: request.catalog.revision,
            results,
            status,
        };
        let _ = request.response.send(Ok(response));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    #[test]
    fn query_worker_rejects_late_older_submissions() -> Result<(), LauncherError> {
        let worker = QueryWorker::new()?;
        let catalog = Catalog::default().snapshot()?;
        tauri::async_runtime::block_on(worker.submit("quit".to_string(), 2, Arc::clone(&catalog)))?;
        let stale = tauri::async_runtime::block_on(worker.submit("quit".to_string(), 1, catalog));
        assert_eq!(
            stale.err().map(|error| error.code).as_deref(),
            Some("stale_result")
        );
        Ok(())
    }
}
