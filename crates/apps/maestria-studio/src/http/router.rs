use std::time::Duration;

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, header},
    middleware,
    routing::{get, post},
};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    set_header::SetResponseHeaderLayer, timeout::RequestBodyTimeoutLayer, trace::TraceLayer,
};

use super::{
    ask, assets, auth, bootstrap, drafts,
    error::{ProblemCode, StudioError},
    evidence, index, notebooks, repository_index, retrieval, search, sources,
    state::StudioState,
    tasks,
};

pub fn build_router(state: StudioState) -> Router {
    let api = Router::new()
        .route("/bootstrap", get(bootstrap::get))
        .route("/notebooks", get(notebooks::list).post(notebooks::create))
        .route(
            "/notebooks/{notebook_id}",
            get(notebooks::get)
                .patch(notebooks::rename)
                .delete(notebooks::delete),
        )
        .route("/notebooks/{notebook_id}/sources", get(sources::catalog))
        .route(
            "/notebooks/{notebook_id}/sources/{source_key}",
            post(sources::attach).delete(sources::detach),
        )
        .route("/notebooks/{notebook_id}/ask", post(ask::ask))
        .route(
            "/notebooks/{notebook_id}/drafts",
            get(drafts::list).post(drafts::create),
        )
        .route(
            "/notebooks/{notebook_id}/drafts/{draft_id}",
            get(drafts::get)
                .patch(drafts::update)
                .delete(drafts::delete),
        )
        .route(
            "/notebooks/{notebook_id}/evidence/{evidence_id}",
            get(evidence),
        )
        .route("/search", get(search::search))
        .route("/evidence/{evidence_id}", get(evidence::evidence_global))
        .route("/retrieval", get(retrieval::status))
        .route("/tasks", get(tasks::list))
        .route("/index/candidates/{root}", get(index::candidates))
        .route(
            "/index/selection",
            get(index::selection_get).put(index::selection_save),
        )
        .route("/index/run", post(index::run))
        .route(
            "/repository-index/candidates/{root}",
            get(repository_index::candidates),
        )
        .route(
            "/repository-index/selection",
            get(repository_index::selection_get).put(repository_index::selection_save),
        )
        .route("/repository-index/run", post(repository_index::run))
        .route(
            "/repository-index/status/{root}",
            get(repository_index::status),
        )
        .route(
            "/repository-index/children",
            post(repository_index::children),
        )
        .route("/repository-index/files", post(repository_index::files))
        .route(
            "/repository-index/progress",
            get(repository_index::progress),
        )
        .fallback(api_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .fallback(assets::frontend)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state.clone())
        .layer(DefaultBodyLimit::max(1_048_576))
        .layer(RequestBodyTimeoutLayer::new(Duration::from_secs(5)))
        .layer(TraceLayer::new_for_http())
        .layer(ConcurrencyLimitLayer::new(32))
        .layer(middleware::from_fn_with_state(state, auth::security))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; \
                 img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; \
                 frame-ancestors 'none'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
}

async fn evidence(
    State(state): State<StudioState>,
    super::extract::ApiPath((notebook_id, evidence_id)): super::extract::ApiPath<(u64, u64)>,
) -> Result<axum::Json<maestria_daemon::api::ClientResponse>, StudioError> {
    let response = state
        .client
        .request(maestria_daemon::api::ClientOperation::NotebookEvidence {
            notebook_id,
            evidence_id,
        })
        .await?;
    Ok(axum::Json(response))
}

async fn api_not_found() -> StudioError {
    StudioError::new(ProblemCode::NotFound)
}

async fn method_not_allowed() -> StudioError {
    StudioError::new(ProblemCode::MethodNotAllowed)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use maestria_daemon::api::DaemonClient;
    use tower::ServiceExt;

    use super::{StudioState, build_router};
    static NEXT_TOKEN_PATH: AtomicUsize = AtomicUsize::new(0);

    use crate::{agent::AgentHost, agent::AgentProfile};

    fn test_router() -> Result<(Router, PathBuf), Box<dyn std::error::Error>> {
        let sequence = NEXT_TOKEN_PATH.fetch_add(1, Ordering::Relaxed);
        let token_path = std::env::temp_dir().join(format!(
            "maestria-studio-router-test-{}-{sequence}.token",
            std::process::id()
        ));
        std::fs::write(
            &token_path,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )?;
        let client =
            DaemonClient::new(PathBuf::from("/tmp/maestria-test.sock"), token_path.clone())?;
        let state = StudioState {
            client,
            agent: AgentHost::new(AgentProfile::default()),
            bearer: Arc::from("test-bearer"),
            origin: Arc::from("http://127.0.0.1:4242"),
        };
        Ok((build_router(state), token_path))
    }

    async fn send(
        router: Router,
        request: Request<Body>,
    ) -> Result<axum::response::Response, Box<dyn std::error::Error>> {
        Ok(router.oneshot(request).await?)
    }

    #[tokio::test]
    async fn static_assets_are_public_and_hardened() -> Result<(), Box<dyn std::error::Error>> {
        let (router, token_path) = test_router()?;
        let response = send(router.clone(), Request::get("/").body(Body::empty())?).await?;
        let fallback = send(
            router,
            Request::get("/notebooks/1/ask").body(Body::empty())?,
        )
        .await?;
        assert_eq!(fallback.status(), StatusCode::OK);
        assert_eq!(
            fallback
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        std::fs::remove_file(token_path)?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        assert!(
            response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        Ok(())
    }

    #[tokio::test]
    async fn api_authentication_and_origin_order_are_typed()
    -> Result<(), Box<dyn std::error::Error>> {
        let (router, token_path) = test_router()?;
        let unauthorized = send(
            router.clone(),
            Request::get("/api/bootstrap").body(Body::empty())?,
        )
        .await?;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let unauthorized_content_type = unauthorized
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let unauthorized_body = to_bytes(unauthorized.into_body(), usize::MAX).await?;
        let unauthorized_json: serde_json::Value = serde_json::from_slice(&unauthorized_body)?;
        assert_eq!(
            unauthorized_json["type"],
            "urn:maestria:studio:problem:unauthorized"
        );
        assert_eq!(
            unauthorized_content_type.as_deref(),
            Some("application/problem+json")
        );

        let origin_denied = send(
            router,
            Request::post("/api/notebooks")
                .header(header::AUTHORIZATION, "Bearer test-bearer")
                .body(Body::from("{}"))?,
        )
        .await?;
        std::fs::remove_file(token_path)?;
        assert_eq!(origin_denied.status(), StatusCode::FORBIDDEN);
        let origin_body = to_bytes(origin_denied.into_body(), usize::MAX).await?;
        let origin_json: serde_json::Value = serde_json::from_slice(&origin_body)?;
        assert_eq!(
            origin_json["type"],
            "urn:maestria:studio:problem:origin-denied"
        );
        Ok(())
    }

    #[tokio::test]
    async fn retrieval_status_requires_authentication() -> Result<(), Box<dyn std::error::Error>> {
        let (router, token_path) = test_router()?;
        for path in ["/api/retrieval", "/api/tasks", "/api/search?query=test"] {
            let unauthorized =
                send(router.clone(), Request::get(path).body(Body::empty())?).await?;
            assert_eq!(
                unauthorized.status(),
                StatusCode::UNAUTHORIZED,
                "expected 401 for GET {path}"
            );
        }
        std::fs::remove_file(token_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn authenticated_invalid_json_uses_problem_details()
    -> Result<(), Box<dyn std::error::Error>> {
        let (router, token_path) = test_router()?;
        let response = send(
            router,
            Request::post("/api/notebooks")
                .header(header::AUTHORIZATION, "Bearer test-bearer")
                .header(header::ORIGIN, "http://127.0.0.1:4242")
                .body(Body::from("{"))?,
        )
        .await?;
        std::fs::remove_file(token_path)?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/problem+json")
        );
        Ok(())
    }
}
