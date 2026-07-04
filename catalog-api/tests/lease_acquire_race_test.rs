//! Proves the split-brain fix: two `KubeLeaseElector`s racing to take over the SAME expired
//! lease cannot both win.
//!
//! There is no real (or fake-server-harness) k8s API available in this sandbox, so this test
//! stands up a minimal fake `coordination.k8s.io/v1` Lease backend as a `tower::Service` and
//! wires it into two real `kube::Client`s that share the same in-memory lease state -- this
//! exercises the actual production code path (`KubeLeaseElector::tick` -> `acquire_or_renew`,
//! including the real JSON (de)serialization of `Lease` and the real `kube`-crate HTTP
//! request-building for GET/PUT), not a reimplementation of its logic.
//!
//! The fake backend implements exactly the semantics a real k8s API server guarantees for
//! `Api::replace` (PUT): the write only succeeds if the request body's
//! `metadata.resourceVersion` still matches the stored value, otherwise it returns 409
//! Conflict. To deterministically force the race (rather than relying on incidental task
//! scheduling), the fake backend makes both GETs rendezvous on a barrier before either
//! responds, guaranteeing both electors observe the identical pre-acquire state before racing
//! their PUTs against each other.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use catalog_api_lib::leader::{KubeLeaseElector, LeaderElector};
use http::{Method, Request, Response};
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::ObjectMeta;
use kube::client::Body;
use kube::core::response::Status;
use kube::Client;
use tokio::sync::{Barrier, Mutex};

const LEASE_NAME: &str = "catalog-api-leader";
const NAMESPACE: &str = "default";

/// In-memory state for the fake Lease backend, shared across both fake `Client`s under test.
struct FakeLeaseBackend {
    lease: Mutex<Option<Lease>>,
    /// Both electors' GETs wait here so neither can race ahead and PUT before the other has
    /// even observed the lease -- this is what makes the split-brain race deterministic
    /// instead of depending on incidental tokio task scheduling.
    get_barrier: Barrier,
}

fn status_response(code: u16, reason: &str) -> Response<Body> {
    let status = Status {
        code,
        reason: reason.to_string(),
        message: format!("fake lease backend: {reason}"),
        ..Default::default()
    };
    Response::builder()
        .status(code)
        .body(Body::from(serde_json::to_vec(&status).unwrap()))
        .unwrap()
}

async fn handle(
    backend: Arc<FakeLeaseBackend>,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let is_lease_object_path = path.ends_with(&format!("/leases/{LEASE_NAME}"));

    match (method, is_lease_object_path) {
        (Method::GET, true) => {
            // Rendezvous with the other racer before answering, so both GETs observe the
            // lease in the same pre-acquire state.
            backend.get_barrier.wait().await;
            let guard = backend.lease.lock().await;
            match &*guard {
                Some(lease) => Ok(Response::builder()
                    .status(200)
                    .body(Body::from(serde_json::to_vec(lease).unwrap()))
                    .unwrap()),
                None => Ok(status_response(404, "NotFound")),
            }
        }
        (Method::PUT, true) => {
            let body_bytes = req.into_body().collect_bytes().await.unwrap();
            let incoming: Lease = serde_json::from_slice(&body_bytes).unwrap();
            let mut guard = backend.lease.lock().await;
            let cas_ok = matches!(
                &*guard,
                Some(current) if current.metadata.resource_version == incoming.metadata.resource_version
            );
            if cas_ok {
                let current_rv: u64 = guard
                    .as_ref()
                    .and_then(|l| l.metadata.resource_version.as_deref())
                    .and_then(|rv| rv.parse().ok())
                    .unwrap_or(0);
                let mut updated = incoming;
                updated.metadata.resource_version = Some((current_rv + 1).to_string());
                *guard = Some(updated.clone());
                Ok(Response::builder()
                    .status(200)
                    .body(Body::from(serde_json::to_vec(&updated).unwrap()))
                    .unwrap())
            } else {
                // Someone else already replaced the lease first -- stale resourceVersion.
                Ok(status_response(409, "Conflict"))
            }
        }
        (Method::POST, false) if path.ends_with("/leases") => {
            let body_bytes = req.into_body().collect_bytes().await.unwrap();
            let incoming: Lease = serde_json::from_slice(&body_bytes).unwrap();
            let mut guard = backend.lease.lock().await;
            if guard.is_some() {
                // Someone else already created the lease first.
                Ok(status_response(409, "AlreadyExists"))
            } else {
                let mut created = incoming;
                created.metadata.resource_version = Some("1".to_string());
                *guard = Some(created.clone());
                Ok(Response::builder()
                    .status(201)
                    .body(Body::from(serde_json::to_vec(&created).unwrap()))
                    .unwrap())
            }
        }
        _ => Ok(status_response(404, "NotFound")),
    }
}

fn client_for(backend: Arc<FakeLeaseBackend>) -> Client {
    let service = tower::service_fn(move |req: Request<Body>| {
        let backend = backend.clone();
        async move { handle(backend, req).await }
    });
    Client::new(service, NAMESPACE)
}

fn expired_lease_held_by(holder_identity: &str) -> Lease {
    let long_ago = k8s_openapi::jiff::Timestamp::now()
        .checked_sub(k8s_openapi::jiff::Span::new().seconds(120))
        .unwrap();
    Lease {
        metadata: ObjectMeta {
            name: Some(LEASE_NAME.to_string()),
            resource_version: Some("1".to_string()),
            ..Default::default()
        },
        spec: Some(LeaseSpec {
            holder_identity: Some(holder_identity.to_string()),
            lease_duration_seconds: Some(10),
            renew_time: Some(MicroTime(long_ago)),
            acquire_time: Some(MicroTime(long_ago)),
            ..Default::default()
        }),
    }
}

#[tokio::test]
async fn two_concurrent_acquirers_of_an_expired_lease_cannot_both_win() {
    let backend = Arc::new(FakeLeaseBackend {
        lease: Mutex::new(Some(expired_lease_held_by("old-holder"))),
        get_barrier: Barrier::new(2),
    });

    let pod_a = KubeLeaseElector::new(
        client_for(backend.clone()),
        NAMESPACE,
        LEASE_NAME,
        "pod-a",
        Duration::from_secs(30),
    );
    let pod_b = KubeLeaseElector::new(
        client_for(backend.clone()),
        NAMESPACE,
        LEASE_NAME,
        "pod-b",
        Duration::from_secs(30),
    );

    let (a_won, b_won) = tokio::join!(pod_a.tick(), pod_b.tick());

    assert_ne!(
        a_won, b_won,
        "exactly one racer must win the takeover of an expired lease, got a={a_won} b={b_won}"
    );
    assert!(a_won || b_won, "one of the two racers must win");

    // The backend's final resourceVersion only ever advanced by one CAS-guarded PUT (the
    // winner's) -- confirms the loser's PUT was rejected, not silently coalesced/ignored.
    let final_rv = backend
        .lease
        .lock()
        .await
        .as_ref()
        .and_then(|l| l.metadata.resource_version.clone())
        .unwrap();
    assert_eq!(final_rv, "2", "only one PUT should have succeeded");
}

#[tokio::test]
async fn two_concurrent_acquirers_of_an_absent_lease_cannot_both_win() {
    // The create-race path (404 -> create, 409 on conflict) was already correct per the
    // reviewer's findings -- this is a confirming regression test, not a new fix.
    let backend = Arc::new(FakeLeaseBackend {
        lease: Mutex::new(None),
        get_barrier: Barrier::new(2),
    });

    let pod_a = KubeLeaseElector::new(
        client_for(backend.clone()),
        NAMESPACE,
        LEASE_NAME,
        "pod-a",
        Duration::from_secs(30),
    );
    let pod_b = KubeLeaseElector::new(
        client_for(backend.clone()),
        NAMESPACE,
        LEASE_NAME,
        "pod-b",
        Duration::from_secs(30),
    );

    let (a_won, b_won) = tokio::join!(pod_a.tick(), pod_b.tick());

    assert_ne!(
        a_won, b_won,
        "exactly one racer must win creation of an absent lease, got a={a_won} b={b_won}"
    );
}
