//! Leader election so only one `catalog-api` pod sweeps + writes the registry at a time.
//!
//! `LeaderElector` is a small trait with a single `tick()` method: "try to acquire/renew
//! leadership, and report `Ok(true)` (we hold it), `Ok(false)` (someone else does), or `Err`
//! (couldn't tell)." A generic `run_leader_election` loop calls `tick()` on an interval and
//! publishes leadership into a shared `Arc<AtomicBool>` read via `is_leader()`.
//!
//! Leadership is deadline-based (see `apply_tick`): a successful renew extends a local deadline
//! by `lease_duration`, and a transient tick failure keeps leadership only until that deadline
//! rather than dropping it on the first blip. This avoids both flapping (one failed kube call
//! demoting a healthy leader and skipping its sweep write) and stale leadership (a hung tick
//! that never resolves keeping the flag `true` forever) — the tick itself is `timeout`-bounded.
//!
//! Two implementations:
//! - [`KubeLeaseElector`] acquires/renews a real `coordination.k8s.io/v1` Lease object
//!   against a k8s API server. This is the production path.
//! - [`ForcedLeaderElector`] always reports a fixed leader/non-leader state with no k8s
//!   dependency at all — used for local dev (`CATALOG_LEADER_MODE=forced-on|forced-off`)
//!   and for tests, so leader-election behavior is exercisable without a real cluster.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, PostParams};
use kube::Client;
use tokio::task::JoinHandle;

use crate::metrics;

/// Shared, cheaply-cloneable leadership flag read by the rest of the service.
pub type LeaderState = Arc<AtomicBool>;

/// Something that can try to acquire/renew leadership on each call and report the outcome.
/// Implementations must be safe to call repeatedly on a timer.
///
/// The `Result<bool>` is three-valued on purpose:
/// - `Ok(true)`  — we hold leadership now (freshly acquired or renewed).
/// - `Ok(false)` — we definitively do NOT hold it (someone else holds a live lease, or we
///   lost an acquire race). A clean negative, not a failure.
/// - `Err(_)`    — the attempt could not be completed (API error, timeout). The caller keeps
///   its prior leadership until its local deadline rather than demoting on a transient blip.
#[async_trait]
pub trait LeaderElector: Send + Sync {
    async fn tick(&self) -> Result<bool>;
}

/// The three leadership states a tick can resolve to, after mapping the elector's
/// `Result<bool>` (and any timeout) onto the deadline policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickResult {
    /// The elector confirmed we hold the lease.
    Leader,
    /// The elector confirmed we do NOT hold the lease (someone else does / lost a race).
    NotLeader,
    /// The tick could not be completed (API error or timeout).
    Transient,
}

/// Given the previous deadline and this tick's result, decide whether we hold leadership now
/// and what the new deadline is.
///
/// Pure (no I/O, no clock reads): `tick_start`/`now` are passed in so the deadline policy is
/// unit-testable. `tick_start` is captured *before* the elector call, so the resulting deadline
/// (`tick_start + lease_duration`) is conservatively *earlier* than the lease's true expiry
/// (the lease's `renewTime` is stamped after the network round-trip), guaranteeing we demote
/// before another pod can legitimately take over.
///
/// The key behavior: a `Transient` result does NOT immediately demote a live leader (that was
/// the flapping bug — one blipped kube call dropped leadership and skipped the sweep write).
/// Leadership is held until the deadline set by the last successful renew, then released.
fn apply_tick(
    prev_deadline: Option<Instant>,
    result: TickResult,
    tick_start: Instant,
    now: Instant,
    lease_duration: Duration,
) -> (bool, Option<Instant>) {
    match result {
        TickResult::Leader => (true, Some(tick_start + lease_duration)),
        TickResult::NotLeader => (false, None),
        TickResult::Transient => {
            if prev_deadline.is_some_and(|deadline| now < deadline) {
                (true, prev_deadline)
            } else {
                (false, None)
            }
        }
    }
}

/// Deterministic-but-varying jitter in `[0, interval/4)` to keep replicas from ticking in
/// lockstep. Zero-dependency: hashes a wall-clock nanosecond sample.
fn tick_jitter(interval: Duration) -> Duration {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    let frac = (hasher.finish() % 1000) as f64 / 1000.0;
    interval.mul_f64(0.25 * frac)
}

/// Spawn a background task that calls `elector.tick()` every `interval` and publishes leadership
/// into `state`, applying a deadline policy so a transient kube error does not demote a live
/// leader (see `apply_tick`). Each tick is bounded by a `timeout` of one `interval` so a hung
/// API call is treated as transient rather than blocking the loop. Runs until the process exits.
pub fn run_leader_election(
    elector: Arc<dyn LeaderElector>,
    state: LeaderState,
    interval: Duration,
    lease_duration: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut deadline: Option<Instant> = None;
        loop {
            let tick_start = Instant::now();
            let result = match tokio::time::timeout(interval, elector.tick()).await {
                Ok(Ok(true)) => TickResult::Leader,
                Ok(Ok(false)) => TickResult::NotLeader,
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "leader tick failed; holding leadership until deadline");
                    TickResult::Transient
                }
                Err(_elapsed) => {
                    tracing::warn!("leader tick timed out; holding leadership until deadline");
                    TickResult::Transient
                }
            };
            let (leader, next_deadline) =
                apply_tick(deadline, result, tick_start, Instant::now(), lease_duration);
            deadline = next_deadline;
            publish_leadership(&state, leader);
            tokio::time::sleep(interval + tick_jitter(interval)).await;
        }
    })
}

/// Publish `leader` into the shared flag, counting a `catalog_leader_transitions_total` edge
/// whenever the value actually changes.
fn publish_leadership(state: &LeaderState, leader: bool) {
    let previous = state.swap(leader, Ordering::SeqCst);
    if previous != leader {
        metrics::record_leader_transition();
        tracing::info!(leader, "leadership state changed");
    }
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
    async fn tick(&self) -> Result<bool> {
        Ok(self.leader)
    }
}

/// What `acquire_or_renew` should do given the lease its `GET` just observed. Pure
/// classification, no I/O -- kept separate from `acquire_or_renew` so the decision logic is
/// unit-testable without a k8s API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseDecision {
    /// We already hold a live (non-expired) lease: renew it. Like `Acquire`, the renew
    /// path is CAS-guarded via `Api::replace` carrying the observed `resourceVersion`
    /// (see `acquire_or_renew`'s `Renew` branch) -- a 409 here means another pod already
    /// updated the lease and we step down rather than assume success.
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
    ///
    /// Expiry is checked BEFORE ownership: a lease that has expired routes through `Acquire`
    /// regardless of whose `holder_identity` it still carries. This matters for a "lapsed
    /// former holder" -- a pod that WAS the holder, then hung/paused for >= lease_duration
    /// (GC pause, scheduling delay, partition heal) before resuming. Such a pod still sees
    /// itself as `holder_identity` in the lease it GETs, but its own lease has genuinely
    /// expired; if it were classified `Renew` here it would take the plain-patch path and
    /// could clobber a legitimate new holder that already won the lease via the CAS-guarded
    /// `Acquire` path (see the split-brain bug this fixes, recorded in `acquire_or_renew`'s
    /// `Renew` branch). Routing every expired lease -- ours or not -- through `Acquire` means
    /// only a genuinely live, currently-held-by-us lease is ever renewed.
    fn decide(&self, existing: &Lease) -> LeaseDecision {
        if self.lease_expired(existing) {
            LeaseDecision::Acquire
        } else if self.we_hold_it(existing) {
            LeaseDecision::Renew
        } else {
            LeaseDecision::BackOff
        }
    }

    async fn acquire_or_renew(&self) -> Result<bool> {
        match self.api.get(&self.lease_name).await {
            Ok(existing) => match self.decide(&existing) {
                // We already hold it AND it hasn't expired (see `decide`'s ordering): still
                // CAS-guarded via `replace` carrying the observed `resourceVersion`, exactly
                // like `Acquire`. This used to be a plain unconditioned `Patch::Apply` (server-
                // side apply, single shared field manager, no resourceVersion precondition) --
                // that was the second split-brain bug: a lapsed former holder whose own lease
                // had actually expired could still classify as `Renew` under the old
                // hold-before-expiry ordering and reassert its holder_identity over a
                // concurrent legitimate takeover, since SSA never conflicts. Now that `decide`
                // routes every expired lease through `Acquire`, only a genuinely live renewal
                // by the true current holder reaches this branch -- but it is CAS-guarded
                // anyway (belt-and-suspenders) so a 409 here (lost to a concurrent write we
                // didn't expect) makes us step down rather than blindly assume success.
                LeaseDecision::Renew => {
                    let mut candidate = self.renewed_spec();
                    candidate.metadata.resource_version = existing.metadata.resource_version;
                    // Preserve the original `acquireTime` across renewals (k8s Lease
                    // convention: acquireTime marks first acquisition, renewTime the last
                    // renewal). `renewed_spec` defaults acquireTime to now for a fresh acquire;
                    // on a renew we carry the existing value forward instead.
                    if let Some(existing_acquire) =
                        existing.spec.as_ref().and_then(|s| s.acquire_time.clone())
                    {
                        if let Some(spec) = candidate.spec.as_mut() {
                            spec.acquire_time = Some(existing_acquire);
                        }
                    }
                    match self
                        .api
                        .replace(&self.lease_name, &PostParams::default(), &candidate)
                        .await
                    {
                        Ok(_) => Ok(true),
                        // Superseded: someone else already changed the lease. Step down
                        // instead of assuming our renewal succeeded.
                        Err(kube::Error::Api(err)) if err.code == 409 => Ok(false),
                        Err(e) => Err(e).context("replace lease to renew"),
                    }
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
    async fn tick(&self) -> Result<bool> {
        // Propagate errors instead of swallowing them: the election loop distinguishes a
        // transient failure (hold leadership until the deadline) from a definitive
        // `Ok(false)` (someone else holds the lease -> demote now). See `run_leader_election`.
        self.acquire_or_renew().await
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
    async fn decide_renews_when_we_hold_it_and_it_has_not_expired() {
        let elector = test_elector("pod-a");
        let lease = lease_with(Some("pod-a"), 1, 30);
        assert_eq!(elector.decide(&lease), LeaseDecision::Renew);
    }

    #[tokio::test]
    async fn decide_acquires_when_we_hold_it_but_our_own_lease_has_expired() {
        // Lapsed-former-holder case: we're still `holder_identity`, but our own lease has
        // expired (we hung/paused >= lease_duration). Expiry must win over "we hold it" so
        // this routes through the CAS-guarded Acquire path, not the old Renew path.
        let elector = test_elector("pod-a");
        let lease = lease_with(Some("pod-a"), 100, 10);
        assert_eq!(elector.decide(&lease), LeaseDecision::Acquire);
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
        assert!(leader.tick().await.unwrap());
        assert!(!non_leader.tick().await.unwrap());
    }

    #[tokio::test]
    async fn run_leader_election_publishes_tick_result_into_shared_state() {
        let state: LeaderState = Arc::new(AtomicBool::new(false));
        let elector: Arc<dyn LeaderElector> = Arc::new(ForcedLeaderElector::new(true));
        let handle = run_leader_election(
            elector,
            state.clone(),
            Duration::from_millis(10),
            Duration::from_secs(30),
        );

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(is_leader(&state));
        handle.abort();
    }

    // --- deadline policy (`apply_tick`) ---------------------------------------------------
    //
    // These drive the pure deadline logic with synthetic `Instant`s, so the "hold leadership
    // through transient errors, but not past the lease deadline" behavior is tested
    // deterministically without spawning the loop or sleeping on the wall clock.

    #[test]
    fn apply_tick_leader_result_sets_a_fresh_deadline() {
        let start = Instant::now();
        let (leader, deadline) = apply_tick(
            None,
            TickResult::Leader,
            start,
            start,
            Duration::from_secs(30),
        );
        assert!(leader);
        assert_eq!(deadline, Some(start + Duration::from_secs(30)));
    }

    #[test]
    fn apply_tick_not_leader_result_demotes_immediately_and_clears_deadline() {
        let start = Instant::now();
        // Even while a prior deadline is still in the future, a definitive NotLeader demotes.
        let prev = Some(start + Duration::from_secs(30));
        let (leader, deadline) = apply_tick(
            prev,
            TickResult::NotLeader,
            start,
            start,
            Duration::from_secs(30),
        );
        assert!(!leader);
        assert_eq!(deadline, None);
    }

    #[test]
    fn apply_tick_transient_holds_leadership_while_within_the_deadline() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(30);
        // `now` is before the deadline: a transient failure must NOT demote a live leader.
        let now = start + Duration::from_secs(5);
        let (leader, next) = apply_tick(
            Some(deadline),
            TickResult::Transient,
            start,
            now,
            Duration::from_secs(30),
        );
        assert!(
            leader,
            "must stay leader through a transient error within the deadline"
        );
        assert_eq!(
            next,
            Some(deadline),
            "deadline must not be extended by a transient tick"
        );
    }

    #[test]
    fn apply_tick_transient_demotes_once_the_deadline_has_passed() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(30);
        let now = start + Duration::from_secs(31); // past the deadline
        let (leader, next) = apply_tick(
            Some(deadline),
            TickResult::Transient,
            start,
            now,
            Duration::from_secs(30),
        );
        assert!(
            !leader,
            "must demote once the lease deadline has elapsed with no renewal"
        );
        assert_eq!(next, None);
    }

    #[test]
    fn apply_tick_transient_with_no_prior_deadline_is_not_leader() {
        let start = Instant::now();
        // Never established leadership (deadline None) + a transient failure -> stay non-leader.
        let (leader, next) = apply_tick(
            None,
            TickResult::Transient,
            start,
            start,
            Duration::from_secs(30),
        );
        assert!(!leader);
        assert_eq!(next, None);
    }
}
