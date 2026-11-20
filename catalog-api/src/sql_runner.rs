//! Runs SQL queries on a runtime of their own, so query work never occupies the threads that
//! answer catalog requests. Admits each query under a global and a per-user cap and lets its
//! owner cancel it by id.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::http::HeaderMap;
use lance_namespace::{Error, NamespaceError};
use tokio::runtime::Runtime;
use tokio::task::{AbortHandle, JoinHandle};

use crate::config::SqlConfig;

/// Set by Google IAP on every request it forwards, as `accounts.google.com:<email>`. Trusted
/// only because IAP fronts every production request.
const IAP_USER_HEADER: &str = "x-goog-authenticated-user-email";
const IAP_USER_PREFIX: &str = "accounts.google.com:";
/// The one user every request without an IAP identity counts as.
pub const ANONYMOUS: &str = "anonymous";

/// The user a request runs as.
pub fn user(headers: &HeaderMap) -> String {
    headers
        .get(IAP_USER_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let value = value.trim();
            value.strip_prefix(IAP_USER_PREFIX).unwrap_or(value)
        })
        .filter(|user| !user.is_empty())
        .unwrap_or(ANONYMOUS)
        .to_string()
}

pub struct SqlRunner {
    /// Always `Some` until drop.
    runtime: Option<Runtime>,
    max_concurrent: usize,
    max_per_user: usize,
    running: Arc<Mutex<Running>>,
}

#[derive(Default)]
struct Running {
    next_token: u64,
    queries: HashMap<u64, Query>,
}

struct Query {
    user: String,
    /// The id the client chose, which cancels the query.
    id: Option<String>,
    abort: AbortHandle,
}

impl SqlRunner {
    pub fn new(config: &SqlConfig) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(config.threads)
            .thread_name("sql-query")
            .enable_all()
            .build()?;
        Ok(Self {
            runtime: Some(runtime),
            max_concurrent: config.max_concurrent,
            max_per_user: config.max_per_user,
            running: Arc::default(),
        })
    }

    /// Admits `query` for `user` and spawns it on the query runtime, or rejects it at once with
    /// Throttling when the user or the instance is at its cap.
    pub fn start<F>(
        &self,
        user: &str,
        query_id: Option<String>,
        query: F,
    ) -> Result<RunningQuery<F::Output>, Error>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let mut running = lock(&self.running);
        let total = running.queries.len();
        let of_user = running.queries.values().filter(|q| q.user == user).count();
        let rejection = if of_user >= self.max_per_user {
            Some(format!(
                "you already have {} queries running; wait for one or cancel it",
                self.max_per_user
            ))
        } else if total >= self.max_concurrent {
            Some(format!(
                "the server is running its maximum of {} queries; try again shortly",
                self.max_concurrent
            ))
        } else {
            None
        };
        if let Some(message) = rejection {
            tracing::info!(
                user,
                running = total,
                of_user,
                "sql query rejected: {message}"
            );
            return Err(NamespaceError::Throttling { message }.into());
        }
        if let Some(id) = &query_id {
            if running
                .queries
                .values()
                .any(|q| q.user == user && q.id.as_ref() == Some(id))
            {
                return Err(NamespaceError::InvalidInput {
                    message: format!("query {id:?} is already running"),
                }
                .into());
            }
        }
        let handle = self
            .runtime
            .as_ref()
            .expect("runtime lives until drop")
            .spawn(query);
        let token = running.next_token;
        running.next_token += 1;
        tracing::info!(
            user,
            query_id,
            running = total + 1,
            of_user = of_user + 1,
            "sql query admitted"
        );
        running.queries.insert(
            token,
            Query {
                user: user.to_string(),
                id: query_id,
                abort: handle.abort_handle(),
            },
        );
        Ok(RunningQuery {
            handle,
            _registration: Registration {
                running: self.running.clone(),
                token,
            },
        })
    }

    /// Aborts `user`'s running query `query_id` and frees its slot; false when `user` runs no
    /// such query.
    pub fn cancel(&self, user: &str, query_id: &str) -> bool {
        let mut running = lock(&self.running);
        let token = running
            .queries
            .iter()
            .find(|(_, q)| q.user == user && q.id.as_deref() == Some(query_id))
            .map(|(token, _)| *token);
        match token.and_then(|token| running.queries.remove(&token)) {
            Some(query) => {
                query.abort.abort();
                true
            }
            None => false,
        }
    }
}

impl Drop for SqlRunner {
    fn drop(&mut self) {
        // Dropping a runtime waits for its tasks, which panics inside the async context the
        // router is dropped in.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// An admitted query. Dropping it, as when the client disconnects, aborts the query and frees
/// its slot.
pub struct RunningQuery<T> {
    handle: JoinHandle<T>,
    _registration: Registration,
}

impl<T> RunningQuery<T> {
    /// The query's output; InvalidInput when it was cancelled.
    pub async fn finish(mut self) -> Result<T, Error> {
        match (&mut self.handle).await {
            Ok(output) => Ok(output),
            Err(e) if e.is_cancelled() => Err(NamespaceError::InvalidInput {
                message: "query cancelled".into(),
            }
            .into()),
            Err(e) => Err(NamespaceError::Internal {
                message: format!("query failed: {e}"),
            }
            .into()),
        }
    }
}

struct Registration {
    running: Arc<Mutex<Running>>,
    token: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let query = lock(&self.running).queries.remove(&self.token);
        if let Some(query) = query {
            query.abort.abort();
        }
    }
}

fn lock(running: &Mutex<Running>) -> MutexGuard<'_, Running> {
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn user_comes_from_the_iap_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(user(&headers), ANONYMOUS);
        headers.insert(
            IAP_USER_HEADER,
            HeaderValue::from_static("accounts.google.com:ada@example.com"),
        );
        assert_eq!(user(&headers), "ada@example.com");
        headers.insert(
            IAP_USER_HEADER,
            HeaderValue::from_static("accounts.google.com:"),
        );
        assert_eq!(user(&headers), ANONYMOUS);
    }

    #[tokio::test]
    async fn dropping_a_running_query_aborts_it() {
        let runner = SqlRunner::new(&SqlConfig::default()).unwrap();
        let (sender, dropped) = tokio::sync::oneshot::channel::<()>();
        let query = runner
            .start("ada", Some("q1".into()), async move {
                let _sender = sender;
                std::future::pending::<()>().await
            })
            .unwrap();
        drop(query);
        // The task drops its future, and the sender with it, once the abort lands.
        tokio::time::timeout(Duration::from_secs(5), dropped)
            .await
            .expect("the query was not aborted")
            .unwrap_err();
        assert!(!runner.cancel("ada", "q1"));
        assert!(lock(&runner.running).queries.is_empty());
    }
}
