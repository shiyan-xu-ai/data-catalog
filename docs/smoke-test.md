# Smoke Test Procedure — kind / local overlay

This procedure validates the full local stack on a real kind cluster:
`catalog-api` + MinIO + Prometheus + Grafana. Run this after the first
`tilt up` completes and all resources show green.

## Prerequisites

```bash
# Install kind and Tilt if not present
brew install kind tilt-dev/tap/tilt   # macOS
# or follow https://kind.sigs.k8s.io and https://docs.tilt.dev/install.html

# Create the kind cluster
kind create cluster --name catalog-dev

# Boot the stack
tilt up
# Wait for all resources to turn green in the Tilt web UI (http://localhost:10350)
```

## 1. Health check

```bash
# catalog-api /healthz must return 200
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:9090/healthz
# expected: 200
```

## 2. Wait for first sweep

The local overlay sets `CATALOG_SWEEP_INTERVAL_SECS=30`. After 30 s the leader
pod runs a sweep against the fixture tables written by `catalog-fixture-init`.

```bash
# Watch sweep metrics — wait for catalog_sweep_tables_checked_total to increment
curl -s http://localhost:9090/metrics | grep catalog_sweep_tables_checked
# expected: catalog_sweep_tables_checked_total{probe_result="..."} > 0
```

## 3. Verify registry populated

```bash
# List tables via the REST API
curl -s http://localhost:8080/v1/tables | python3 -m json.tool
# expected: JSON array containing "smoke_test" and "closed_loop_run"

# Describe a table
curl -s http://localhost:8080/v1/table/smoke_test | python3 -m json.tool
# expected: JSON with versions list; smoke_test has version "2024-01-01T00:00:00"
```

## 4. TTL dry-run

```bash
# Dry-run TTL for smoke_test (no policy set yet — expect empty eligible list)
curl -s "http://localhost:8080/ext/v1/tables/smoke_test/ttl/dryrun" | python3 -m json.tool

# Set a TTL policy (keep last 5, max age 0 s = delete everything beyond keep_count)
curl -s -X PUT http://localhost:8080/v1/table/smoke_test \
  -H 'Content-Type: application/json' \
  -d '{"ttl_policy": {"keep_last_n": 1, "max_age_secs": null}}' \
  | python3 -m json.tool

# Dry-run again — smoke_test has 1 version so nothing should be eligible
# (only eligible if keep_last_n < version count)
curl -s "http://localhost:8080/ext/v1/tables/smoke_test/ttl/dryrun" | python3 -m json.tool
```

## 5. TTL apply (hard delete)

> **Warning**: TTL apply is irreversible. The fixture data is disposable, but
> double-check the `eligible_versions` list in the dry-run before applying.

```bash
# Only the leader pod applies — returns 503 on followers.
# With 1 replica in the local overlay the single pod is always the leader.
curl -s -X POST http://localhost:8080/ext/v1/tables/smoke_test/ttl/apply \
  | python3 -m json.tool
# expected: {"deleted_versions": [...], "failed_versions": []}
```

## 6. Verify delete propagated

```bash
# After apply, the deleted version should be gone from the registry.
# Wait one sweep cycle (30 s) and re-check.
sleep 35
curl -s http://localhost:8080/v1/table/smoke_test | python3 -m json.tool
# expected: versions list no longer contains the deleted timestamp
```

## 7. KubeLeaseElector end-to-end validation

> This step validates the leader-election behavior that unit tests cover against
> a fake backend. A real kind cluster exercises server-side etcd resourceVersion
> semantics.

```bash
# Scale to 2 replicas so both leader election and standby are exercised.
kubectl scale deployment catalog-api --replicas=2
kubectl rollout status deployment/catalog-api

# Find the current leader (look for "acquired" in logs)
kubectl logs -l app=catalog-api --tail=50 | grep -i "leader\|acquire\|renew"

# Kill the leader pod and confirm the standby takes over within one lease duration.
# The base overlay sets CATALOG_LEASE_DURATION_SECS=30, so failover < 30 s.
LEADER_POD=$(kubectl get pods -l app=catalog-api -o jsonpath='{.items[0].metadata.name}')
kubectl delete pod "$LEADER_POD"
# Watch: within 30 s the remaining pod should log "acquired lease"
kubectl logs -l app=catalog-api --follow --tail=20

# Confirm exactly one is_leader series equals 1
curl -s http://localhost:9090/metrics | grep catalog_is_leader
# expected: catalog_is_leader 1  (on the new leader pod)
```

## 8. MinIO conditional-write conformance

Lance commits depend on `If-None-Match: *` conditional PUT. Verify MinIO honors it:

```bash
# Two concurrent writes to the same key — exactly one must win (412 on the loser).
BUCKET="catalog-data"
KEY="conformance-test/$(date +%s)"

# Write first object
aws --endpoint-url http://localhost:9000 \
    --region us-east-1 \
    s3 cp /dev/stdin "s3://${BUCKET}/${KEY}" <<< "version-1"

# Attempt conditional write (If-None-Match: *) — must fail with 412 because the
# key already exists.
curl -s -w "\nHTTP %{http_code}\n" \
     -X PUT \
     -H "If-None-Match: *" \
     --data "version-2" \
     "http://localhost:9000/${BUCKET}/${KEY}"
# expected HTTP 412 (Precondition Failed)
```

## Teardown

```bash
tilt down
kind delete cluster --name catalog-dev
```
