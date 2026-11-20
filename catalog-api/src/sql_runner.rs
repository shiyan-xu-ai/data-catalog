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
    /// Set once the task is spawned.
    abort: Option<AbortHandle>,
    /// Aborted but still holding its slot until the task drops its future.
    cancelled: bool,
}

impl Query {
    fn cancel(&mut self) {
        self.cancelled = true;
        if let Some(abort) = &self.abort {
            abort.abort();
        }
    }
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
        let token = {
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
                    .any(|q| q.user == user && !q.cancelled && q.id.as_ref() == Some(id))
                {
                    return Err(NamespaceError::InvalidInput {
                        message: format!("query {id:?} is already running"),
                    }
                    .into());
                }
            }
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
                    abort: None,
                    cancelled: false,
                },
            );
            token
        };
        let slot = Slot {
            running: self.running.clone(),
            token,
        };
        // Spawned outside the lock: a future dropped inside `spawn` drops the slot, which locks.
        let handle = self
            .runtime
            .as_ref()
            .expect("runtime lives until drop")
            .spawn(async move {
                // The query drops before the slot, on completion and on abort alike, so the slot
                // frees only once the query's own state is gone.
                let output = query.await;
                drop(slot);
                output
            });
        if let Some(entry) = lock(&self.running).queries.get_mut(&token) {
            entry.abort = Some(handle.abort_handle());
            if entry.cancelled {
                handle.abort();
            }
        }
        Ok(RunningQuery {
            handle,
            _cancel_on_drop: CancelOnDrop {
                running: self.running.clone(),
                token,
            },
        })
    }

    /// Aborts `user`'s running query `query_id`; false when `user` runs no such query or has
    /// already cancelled it. Its slot frees once the task drops the query.
    pub fn cancel(&self, user: &str, query_id: &str) -> bool {
        let mut running = lock(&self.running);
        let query = running
            .queries
            .values_mut()
            .find(|q| q.user == user && !q.cancelled && q.id.as_deref() == Some(query_id));
        match query {
            Some(query) => {
                query.cancel();
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

/// An admitted query. Dropping it, as when the client disconnects, aborts the query; its slot
/// frees once the task drops the query.
pub struct RunningQuery<T> {
    handle: JoinHandle<T>,
    _cancel_on_drop: CancelOnDrop,
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

struct CancelOnDrop {
    running: Arc<Mutex<Running>>,
    token: u64,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(query) = lock(&self.running).queries.get_mut(&self.token) {
            query.cancel();
        }
    }
}

/// Owned by the spawned task, so a query's slot stays taken until tokio drops its future,
/// whether it completed or was aborted.
struct Slot {
    running: Arc<Mutex<Running>>,
    token: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        lock(&self.running).queries.remove(&self.token);
    }
}

fn lock(running: &Mutex<Running>) -> MutexGuard<'_, Running> {
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
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
        assert!(!runner.cancel("ada", "q1"));
        // The task drops its future, and the sender with it, once the abort lands.
        tokio::time::timeout(Duration::from_secs(5), dropped)
            .await
            .expect("the query was not aborted")
            .unwrap_err();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !lock(&runner.running).queries.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the slot was not freed");
    }

    /// Signals when its drop starts, then blocks the drop until released.
    struct SlowDrop {
        dropping: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    impl Drop for SlowDrop {
        fn drop(&mut self) {
            self.dropping.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }

    #[tokio::test]
    async fn a_cancelled_query_holds_its_slot_until_its_task_drops_it() {
        let config = SqlConfig {
            max_per_user: 1,
            ..SqlConfig::default()
        };
        let runner = SqlRunner::new(&config).unwrap();
        let (dropping_tx, dropping) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let slow = SlowDrop {
            dropping: dropping_tx,
            release: release_rx,
        };
        let query = runner
            .start("ada", Some("q1".into()), async move {
                let _slow = slow;
                std::future::pending::<()>().await
            })
            .unwrap();
        assert!(runner.cancel("ada", "q1"));
        assert!(!runner.cancel("ada", "q1"));
        dropping.recv_timeout(Duration::from_secs(5)).unwrap();
        // The aborted task is still dropping the query, so its slot stays taken.
        assert!(runner.start("ada", None, async {}).is_err());
        assert_eq!(lock(&runner.running).queries.len(), 1);
        release.send(()).unwrap();
        let error = query.finish().await.unwrap_err();
        assert!(error.to_string().contains("query cancelled"), "{error}");
        assert!(lock(&runner.running).queries.is_empty());
        runner
            .start("ada", Some("q1".into()), async {})
            .unwrap()
            .finish()
            .await
            .unwrap();
    }
}
