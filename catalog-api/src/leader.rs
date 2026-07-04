//! Leader election so only one `catalog-api` pod sweeps + writes the registry at a time.
//!
//! `LeaderElector` is a small trait with a single `tick()` method: "try to
//! acquire/renew leadership, return whether we hold it now." A generic `run_leader_election`
//! loop calls `tick()` on an interval and publishes the result into a shared
//! `Arc<AtomicBool>` that the rest of the service reads via `is_leader()`.
//!
//! Two implementations:
//! - [`KubeLeaseElector`] acquires/renews a real `coordination.k8s.io/v1` Lease object
//!   against a k8s API server. This is the production path.
//! - [`ForcedLeaderElector`] always reports a fixed leader/non-leader state with no k8s
//!   dependency at all — used for local dev (`CATALOG_LEADER_MODE=forced-on|forced-off`)
//!   and for tests, so leader-election behavior is exercisable without a real cluster.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::Client;
use tokio::task::JoinHandle;

/// Shared, cheaply-cloneable leadership flag read by the rest of the service.
pub type LeaderState = Arc<AtomicBool>;

/// Something that can try to acquire/renew leadership on each call and report whether we
/// hold it right now. Implementations must be safe to call repeatedly on a timer.
#[async_trait]
pub trait LeaderElector: Send + Sync {
    async fn tick(&self) -> bool;
}

/// Spawn a background task that calls `elector.tick()` every `interval` and publishes the
/// result into `state`. Runs until the process exits (no graceful shutdown needed for this
/// phase — the task is a plain infinite loop owned by `main`).
pub fn run_leader_election(
    elector: Arc<dyn LeaderElector>,
    state: LeaderState,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let is_leader = elector.tick().await;
            state.store(is_leader, Ordering::SeqCst);
            tokio::time::sleep(interval).await;
        }
    })
}

pub fn is_leader(state: &LeaderState) -> bool {
    state.load(Ordering::SeqCst)
}

/// Always reports a fixed leadership state. For local dev and tests — no k8s API needed.
pub struct ForcedLeaderElector {
    leader: bool,
}

impl ForcedLeaderElector {
    pub fn new(leader: bool) -> Self {
        Self { leader }
    }
}

#[async_trait]
impl LeaderElector for ForcedLeaderElector {
    async fn tick(&self) -> bool {
        self.leader
    }
}

/// What `acquire_or_renew` should do given the lease its `GET` just observed. Pure
/// classification, no I/O -- kept separate from `acquire_or_renew` so the decision logic is
/// unit-testable without a k8s API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseDecision {
    /// We already hold it: safe to renew with a plain patch, no CAS needed.
    Renew,
    /// Absent/expired and not ours: must take it over via an atomic compare-and-swap.
    Acquire,
    /// Someone else holds a live lease: back off, report non-leader.
    BackOff,
}

/// Acquires/renews a `coordination.k8s.io/v1` Lease. Not a from-scratch reimplementation of
/// every edge case in kube-rs's own `LeaseLock` — this covers the common path: create the
/// Lease if absent, take it if unheld/expired (atomically, via CAS -- see `LeaseDecision::Acquire`),
/// renew it if we already hold it, and back off (report non-leader) if someone else holds a
/// live lease.
pub struct KubeLeaseElector {
    api: Api<Lease>,
    lease_name: String,
    holder_identity: String,
    lease_duration: Duration,
}

impl KubeLeaseElector {
    pub fn new(
        client: Client,
        namespace: &str,
        lease_name: impl Into<String>,
        holder_identity: impl Into<String>,
        lease_duration: Duration,
    ) -> Self {
        Self {
            api: Api::namespaced(client, namespace),
            lease_name: lease_name.into(),
            holder_identity: holder_identity.into(),
            lease_duration,
        }
    }

    /// Build a client from the in-cluster/kubeconfig-inferred environment.
    pub async fn from_env(
        namespace: &str,
        lease_name: impl Into<String>,
        holder_identity: impl Into<String>,
        lease_duration: Duration,
    ) -> Result<Self> {
        let client = Client::try_default()
            .await
            .context("build kube client from environment")?;
        Ok(Self::new(
            client,
            namespace,
            lease_name,
            holder_identity,
            lease_duration,
        ))
    }

    fn lease_expired(&self, lease: &Lease) -> bool {
        let Some(spec) = &lease.spec else {
            return true;
        };
        let Some(renew_time) = &spec.renew_time else {
            return true;
        };
        let duration_secs = spec.lease_duration_seconds.unwrap_or(0) as i64;
        let Ok(expiry) = renew_time
            .0
            .checked_add(k8s_openapi::jiff::Span::new().seconds(duration_secs))
        else {
            return true;
        };
        k8s_openapi::jiff::Timestamp::now() > expiry
    }

    fn we_hold_it(&self, lease: &Lease) -> bool {
        lease
            .spec
            .as_ref()
            .and_then(|s| s.holder_identity.as_deref())
            == Some(self.holder_identity.as_str())
    }

    /// What to do given the lease this GET just observed.
    fn decide(&self, existing: &Lease) -> LeaseDecision {
        if self.we_hold_it(existing) {
            LeaseDecision::Renew
        } else if self.lease_expired(existing) {
            LeaseDecision::Acquire
        } else {
            LeaseDecision::BackOff
        }
    }

    async fn acquire_or_renew(&self) -> Result<bool> {
        match self.api.get(&self.lease_name).await {
            Ok(existing) => match self.decide(&existing) {
                // We already hold it: only the actual holder legitimately renews here (no
                // other pod can be renewing the same lease under our identity), so a plain
                // server-side-apply patch is fine -- no compare-and-swap needed.
                LeaseDecision::Renew => {
                    let patch = self.renewed_spec();
                    self.api
                        .patch(
                            &self.lease_name,
                            &PatchParams::apply("catalog-api-leader-elect"),
                            &Patch::Apply(&patch),
                        )
                        .await
                        .context("patch lease to renew")?;
                    Ok(true)
                }
                // Takeover of an expired/unheld lease MUST be an atomic compare-and-swap:
                // carry the `resourceVersion` this GET just observed into a `replace` (PUT).
                // If another candidate already won the race and updated the lease first, the
                // API server rejects our now-stale-resourceVersion PUT with 409 Conflict -- we
                // lost, report non-leader. `Patch::Apply` (server-side apply) has NO such
                // precondition and same-manager writes never conflict, so it must never be
                // used for this branch (that was the split-brain bug: two candidates racing
                // on an expired lease could both `Patch::Apply` successfully and both become
                // leader).
                LeaseDecision::Acquire => {
                    let mut candidate = self.renewed_spec();
                    candidate.metadata.resource_version = existing.metadata.resource_version;
                    match self
                        .api
                        .replace(&self.lease_name, &PostParams::default(), &candidate)
                        .await
                    {
                        Ok(_) => Ok(true),
                        // Someone else already won the race and updated the lease first.
                        Err(kube::Error::Api(err)) if err.code == 409 => Ok(false),
                        Err(e) => Err(e).context("replace lease to acquire"),
                    }
                }
                // Someone else holds a live lease.
                LeaseDecision::BackOff => Ok(false),
            },
            Err(kube::Error::Api(err)) if err.code == 404 => {
                let lease = self.new_lease();
                match self.api.create(&PostParams::default(), &lease).await {
                    Ok(_) => Ok(true),
                    // Lost the create race to another pod; not leader this tick.
                    Err(kube::Error::Api(err)) if err.code == 409 => Ok(false),
                    Err(e) => Err(e).context("create lease"),
                }
            }
            Err(e) => Err(e).context("get lease"),
        }
    }

    fn renewed_spec(&self) -> Lease {
        Lease {
            metadata: kube::api::ObjectMeta {
                name: Some(self.lease_name.clone()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                holder_identity: Some(self.holder_identity.clone()),
                lease_duration_seconds: Some(self.lease_duration.as_secs() as i32),
                renew_time: Some(MicroTime(k8s_openapi::jiff::Timestamp::now())),
                acquire_time: Some(MicroTime(k8s_openapi::jiff::Timestamp::now())),
                ..Default::default()
            }),
        }
    }

    fn new_lease(&self) -> Lease {
        let mut lease = self.renewed_spec();
        lease.metadata.name = Some(self.lease_name.clone());
        lease
    }
}

#[async_trait]
impl LeaderElector for KubeLeaseElector {
    async fn tick(&self) -> bool {
        match self.acquire_or_renew().await {
            Ok(is_leader) => is_leader,
            Err(e) => {
                tracing::warn!(error = %e, "lease acquire/renew failed; reporting non-leader");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `KubeLeaseElector` whose `api` is never actually called -- used to unit-test the
    /// pure `decide()` classification, which only reads `holder_identity` and the lease
    /// passed in, with no I/O.
    fn test_elector(holder_identity: &str) -> KubeLeaseElector {
        let service = tower::service_fn(|_req: http::Request<kube::client::Body>| async {
            Err::<http::Response<kube::client::Body>, _>(std::io::Error::other(
                "dummy client not wired to a backend",
            ))
        });
        let client = Client::new(service, "default");
        KubeLeaseElector::new(
            client,
            "default",
            "catalog-api-leader",
            holder_identity,
            Duration::from_secs(30),
        )
    }

    fn lease_with(
        holder_identity: Option<&str>,
        renew_seconds_ago: i64,
        duration_secs: i32,
    ) -> Lease {
        let renew_time = k8s_openapi::jiff::Timestamp::now()
            .checked_sub(k8s_openapi::jiff::Span::new().seconds(renew_seconds_ago))
            .unwrap();
        Lease {
            metadata: kube::api::ObjectMeta {
                name: Some("catalog-api-leader".into()),
                resource_version: Some("1".into()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                holder_identity: holder_identity.map(|s| s.to_string()),
                lease_duration_seconds: Some(duration_secs),
                renew_time: Some(MicroTime(renew_time)),
                acquire_time: Some(MicroTime(renew_time)),
                ..Default::default()
            }),
        }
    }

    // `#[tokio::test]`, not plain `#[test]`: `Client::new` (used by `test_elector`) spawns a
    // `tower::buffer::Buffer` worker task, which requires a Tokio runtime to exist even
    // though `decide()` itself performs no I/O.

    #[tokio::test]
    async fn decide_renews_when_we_already_hold_it_even_if_technically_expired() {
        let elector = test_elector("pod-a");
        // We hold it, but haven't renewed in a while -- still ours to renew, not a takeover.
        let lease = lease_with(Some("pod-a"), 100, 10);
        assert_eq!(elector.decide(&lease), LeaseDecision::Renew);
    }

    #[tokio::test]
    async fn decide_acquires_an_expired_lease_held_by_someone_else() {
        let elector = test_elector("pod-b");
        let lease = lease_with(Some("pod-a"), 100, 10);
        assert_eq!(elector.decide(&lease), LeaseDecision::Acquire);
    }

    #[tokio::test]
    async fn decide_backs_off_from_a_live_lease_held_by_someone_else() {
        let elector = test_elector("pod-b");
        let lease = lease_with(Some("pod-a"), 1, 30);
        assert_eq!(elector.decide(&lease), LeaseDecision::BackOff);
    }

    #[tokio::test]
    async fn decide_acquires_an_absent_holder_lease() {
        let elector = test_elector("pod-b");
        let lease = lease_with(None, 100, 10);
        assert_eq!(elector.decide(&lease), LeaseDecision::Acquire);
    }

    #[tokio::test]
    async fn forced_leader_reports_fixed_state() {
        let leader = ForcedLeaderElector::new(true);
        let non_leader = ForcedLeaderElector::new(false);
        assert!(leader.tick().await);
        assert!(!non_leader.tick().await);
    }

    #[tokio::test]
    async fn run_leader_election_publishes_tick_result_into_shared_state() {
        let state: LeaderState = Arc::new(AtomicBool::new(false));
        let elector: Arc<dyn LeaderElector> = Arc::new(ForcedLeaderElector::new(true));
        let handle = run_leader_election(elector, state.clone(), Duration::from_millis(10));

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(is_leader(&state));
        handle.abort();
    }
}
