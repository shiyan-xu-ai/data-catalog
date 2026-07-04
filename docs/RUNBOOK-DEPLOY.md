# Deployment runbook

How to deploy `catalog-api` and the monitoring stack. The kustomize base +
overlays live in `deploy/`; the Tiltfile boots a local kind cluster with the
local overlay.

## Local (kind + Tilt + MinIO)

Prerequisites: [kind](https://kind.sigs.k8s.io/),
[Tilt](https://docs.tilt.dev/install.html), `kubectl`, `aws` CLI.

```sh
kind create cluster --name catalog-dev
tilt up   # http://localhost:10350
```

The local overlay (`deploy/overlays/local`) runs:

- `catalog-api` (1 replica by default; scale to 2 for leader-failover testing).
- MinIO (serves as the sweep root; the fixture-init job writes small Lance
  fixture tables into it on first boot).
- Prometheus + Grafana (the monitoring stack).
- A ConfigMap that sets the `AWS_*` env vars so the sweep and Lance object
  stores reach MinIO (`AWS_ENDPOINT`, `AWS_ALLOW_HTTP=true`,
  `AWS_VIRTUAL_HOSTED_STYLE_REQUEST=false`, region + creds).

Tilt port-forwards the API pod's port 8080 to `localhost:8080` and the
internal metrics/healthz port 9090 to `localhost:9090`.

Smoke-test the running stack by following [`docs/smoke-test.md`](smoke-test.md)
(health, sweep populated the registry, TTL dry-run/apply, leader failover,
MinIO conditional-write conformance).

### Leader-election validation (KubeLeaseElector)

This validates the leader-election behavior that unit tests cover against a
fake backend. A real kind cluster exercises server-side etcd `resourceVersion`
semantics (exact SSA/replace interplay, real etcd resourceVersion semantics)
that the fake backend does not.

```sh
# Scale to 2 replicas so both leader election and standby are exercised.
kubectl scale deployment catalog-api --replicas=2
kubectl rollout status deployment catalog-api

# Find the current leader (look for "acquired"/"leader" in logs).
kubectl logs -l app=catalog-api --tail=50 | grep -i "leader\|acquire\|renew"

# Kill the leader pod and confirm the standby takes over within one lease duration.
# The base overlay sets CATALOG_LEASE_DURATION_SECS=30, so failover < 30 s.
LEADER_POD=$(kubectl get pods -l app=catalog-api -o jsonpath='{.items[0].metadata.name}')
kubectl delete pod "$LEADER_POD"

# Watch: within 30 s the remaining pod should acquire the lease.
kubectl logs -l app=catalog-api --follow --tail=20

# Confirm exactly one catalog_is_leader series equals 1.
curl -s http://localhost:9090/metrics | grep catalog_is_leader
# expected: catalog_is_leader 1  (on the new leader pod)
```

Acceptance: exactly one pod reports `catalog_is_leader 1` within one lease
duration of the leader being killed; the killed pod, once recreated, reports
`catalog_is_leader 0`. No two pods report `1` simultaneously.

This procedure is also step 7 of [`docs/smoke-test.md`](smoke-test.md).

## Staging

`deploy/overlays/staging` targets a staging cluster. It patches the base
ConfigMap with the staging sweep root + registry paths and the staging
service account (IRSA annotation). Deploy with:

```sh
kustomize build deploy/overlays/staging | kubectl apply -f -
```

Review the rendered manifest before applying:

```sh
kustomize build deploy/overlays/staging
```

## Prod

`deploy/overlays/prod` targets production. It adds an HPA (RPS-based), a PDB,
and anti-affinity, and patches the service account with the prod IRSA
annotation. Deploy with:

```sh
kustomize build deploy/overlays/prod | kubectl apply -f -
```

### Prerequisites (prod)

- **IRSA:** the catalog-api pod needs IRSA credentials with read+list+delete
  on the sweep bucket (and read+write on the `_catalog/` registry bucket). The
  base `deploy/base/rbac.yaml` carries the IRSA role stub; the prod overlay
  patches the service account with the role ARN annotation. Fill in the ARN
  in `deploy/overlays/prod/serviceaccount-patch.yaml` before applying.
- **Service + RBAC + Lease:** the base manifest (`deploy/base/`) creates the
  Service (api port 8080 + metrics port 9090), the ServiceAccount, the RBAC
  for the Lease (`coordination.k8s.io` leases get/update/create), and the
  Deployment (2 replicas by default, `CATALOG_LEADER_MODE=kube`,
  `CATALOG_POD_NAME` via the downward API).
- **Monitoring stack:** apply `deploy/base/monitoring/` (Prometheus + Grafana
  + ServiceMonitor + PrometheusRule + dashboards). The ServiceMonitor selects
  the `metrics` named port on the catalog-api Service.
- **Image build + push:** build the image from the repo root `Dockerfile`
  and push to your registry; set the image in `deploy/base/deployment.yaml`
  (or patch it in the prod overlay).

```sh
# Build and push (replace registry/repo)
docker build -t <registry>/catalog-api:1.0.0 .
docker push <registry>/catalog-api:1.0.0

# Apply the monitoring stack
kubectl apply -k deploy/base/monitoring

# Apply the prod overlay
kustomize build deploy/overlays/prod | kubectl apply -f -
```

### Dockerfile note

The builder stage is `FROM rust:1.87-slim`; `rust-toolchain.toml` pins
`channel = "1.94.0"`. rustup honors the toolchain file in-image, so the build
uses 1.94.0, but the base-image tag is stale/misleading — bumping to
`rust:1.94-slim` improves cache-hit clarity. Non-blocking.

## Frontend

The frontend is a static SPA deployed to S3 + CloudFront. The one-time S3 +
CloudFront setup (dedicated bucket, Block Public Access ON, OAI, bucket
policy) and the deploy script are documented in
[`frontend/docs/DEPLOY.md`](../frontend/docs/DEPLOY.md).

```sh
cd frontend
bun install
VITE_API_BASE=https://catalog-api.example.com bun run build   # bake the API base for prod
./scripts/deploy.sh <dedicated-frontend-bucket> <cloudfront-distribution-id>
```

**The deploy script runs `aws s3 sync --delete` against the bucket root.**
`--delete` removes any key in the destination not present in the local
`dist/`. The bucket MUST be a dedicated frontend-only bucket — never point the
script at a bucket that holds anything other than the frontend build output.
The script echoes a confirmation and waits 3s before the destructive sync.
See [`frontend/docs/DEPLOY.md`](../frontend/docs/DEPLOY.md) for the
dedicated-bucket prerequisite.

## Deferred real-cluster validations

The following require a live kind cluster / real S3 / real browser and are
documented as procedures here and in [`docs/smoke-test.md`](smoke-test.md)
rather than run in-sandbox:

- **KubeLeaseElector end-to-end kill-leader** (procedure above; the fake-
  backend unit tests prove the client-side CAS logic).
- **kind `tilt up` boot** — the Tiltfile + local overlay + MinIO + fixture job
  are complete; the actual boot needs docker/kind.
- **Real-S3/MinIO TTL delete conformance** — the TTL delete path is tested
  against `object_store::local::LocalFileSystem`; the S3 LIST-then-delete +
  NotFound tolerance + pagination should be validated once against a real
  MinIO backend (step 8 of `docs/smoke-test.md` covers the Lance
  conditional-write conformance check).
- **Load smoke against the real 65-table sweep root** — verifying pre+post
  cutoff version attribution on `smoke_test` + `closed_loop` at production
  scale.
