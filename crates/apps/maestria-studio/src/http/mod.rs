//! Studio's authenticated loopback HTTP boundary.
//!
//! Responsibility map:
//! - `auth`: loopback origin and bearer authentication middleware.
//! - `router`: Axum routes, middleware, limits, and security order.
//! - `error`: RFC 9457 Problem Details and typed cause mapping.
//! - `extract`: typed API JSON and path rejection mapping.
//! - `assets`: embedded Dioxus assets and MIME boundaries.
//! - `bootstrap`: startup status and notebook bootstrap DTO.
//! - `notebooks`: notebook CRUD handlers.
//! - `sources`: source catalog and selection handlers.
//! - `drafts`: draft CRUD handlers.
//! - `ask`: grounded Ask validation and agent protocol boundary.

mod ask;
mod assets;
mod auth;
mod bootstrap;
mod drafts;
mod error;
mod evidence;
mod extract;
mod notebooks;
mod retrieval;
mod router;
mod search;
mod sources;
mod state;
mod tasks;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub(crate) use router::build_router;
pub(crate) use state::StudioState;

/// Serves Studio until its cancellation token is triggered.
///
/// # Cancellation
///
/// Cancellation allows accepted requests to drain before the task completes. Dropping the
/// shutdown future leaves the server task running; callers must retain and cancel the token.
pub(crate) async fn serve(
    listener: TcpListener,
    state: StudioState,
    shutdown: CancellationToken,
) -> Result<()> {
    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
        .context("serve Studio HTTP")
}
