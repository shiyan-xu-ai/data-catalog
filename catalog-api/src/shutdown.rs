//! Process shutdown signal.
//!
//! Resolves when the process receives `SIGTERM` (a Kubernetes pod termination) or Ctrl-C
//! (`SIGINT`, local dev). Used to drive graceful shutdown of the HTTP servers: each server's
//! `with_graceful_shutdown` awaits its own `shutdown_signal()` future so all of them drain on
//! the same signal.

/// Resolve when a shutdown signal (`SIGTERM` or Ctrl-C) is received.
///
/// Each call registers its own handlers, so it can be awaited independently by more than one
/// server. Installing the `SIGTERM` handler is best-effort: if it fails, only Ctrl-C triggers
/// shutdown.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to install SIGTERM handler; Ctrl-C only");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!("shutdown signal received");
}
