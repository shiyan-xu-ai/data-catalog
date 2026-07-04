# Data Catalog

A Lance data catalog service: an S3-native metadata catalog for Lance tables,
written in Rust (axum + tower, the `lance` crate, `object_store`, `kube-rs`).

The service periodically sweeps an S3 root, discovers timestamp-path versioned
Lance tables, and records per-version metadata (shape, storage size split by
component, row count, schema, aux entries) into a Lance-backed registry under
`_catalog/`. A REST API exposes the catalog (basic ops + an enriched `/ext`
listing) and a per-table TTL engine that hard-deletes old versions under an
explicit API policy. A minimal static SPA (vanilla TS + Vite) is served from
S3 + CloudFront for browsing and TTL operations. Prometheus + Grafana
monitoring manifests ship in-repo.

The full long-term design lives in [`docs/design.md`](docs/design.md); v1.0.0
implements a subset (see [Known limitations](#known-limitations)). The
as-built architecture is documented in [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Workspace layout

Cargo workspace with three crates:

- `catalog-core` — shared types (`TableEntry`, `TableVersion`, `TtlPolicy`,
  `TtlAuditRecord`, `AuxEntry`, `Namespace`, `VersionShape`, `AuxFormat`),
  the Lance registry reader/writer, the idempotent `apply_sweep_result` merge,
  and the TTL eligibility computation.
- `catalog-store` — object-store IO and the S3 sweep (discovery, timestamp-path
  version classification, shape detection, per-component size split, aux format
  detection, pre+post cutoff layout handling) plus the TTL hard-delete helper.
- `catalog-api` — the `catalog-api` binary: the axum HTTP server, the
  leader-elected sweep loop, the TTL engine, leader election via a k8s Lease,
  the registry cache, and Prometheus instrumentation.

Additional directories:

- `frontend/` — vanilla TS + Vite static SPA (table list, drill-down, TTL
  dry-run/apply/audit). See [`frontend/docs/DEPLOY.md`](frontend/docs/DEPLOY.md).
- `deploy/` — kustomize base + overlays (`local`, `staging`, `prod`) and the
  in-repo Prometheus + Grafana monitoring stack. See
  [`docs/RUNBOOK-DEPLOY.md`](docs/RUNBOOK-DEPLOY.md).
- `docs/` — design doc, deploy + TTL runbooks, smoke-test procedure.

## Quickstart (local dev via Tilt + kind + MinIO)

Prerequisites: [kind](https://kind.sigs.k8s.io/),
[Tilt](https://docs.tilt.dev/install.html), `kubectl`, `aws` CLI.

```sh
kind create cluster --name catalog-dev
tilt up   # open http://localhost:10350, wait for resources to turn green
```

The local overlay (`deploy/overlays/local`) runs `catalog-api` (1 replica,
forced- or kube-leader election), a MinIO instance serving as the sweep root,
a fixture-init job that writes small Lance fixture tables into MinIO, and the
Prometheus + Grafana monitoring stack. Tilt port-forwards the API pod's port
8080 to `localhost:8080` and the internal metrics/healthz port 9090 to
`localhost:9090`.

Smoke-test the running stack (health, sweep populated the registry, TTL
dry-run/apply, leader failover) by following
[`docs/smoke-test.md`](docs/smoke-test.md).

To run the frontend against the Tilt stack:

```sh
cd frontend && bun install && bun run dev
# open http://localhost:5173 — the Vite dev proxy forwards /v1 and /ext to localhost:8080
```

### Running the API standalone (no cluster)

For local development without a k8s cluster, `catalog-api` can run against a
local filesystem sweep root with a forced leader state:

```sh
CATALOG_LEADER_MODE=forced-on \
CATALOG_SWEEP_ROOT_URI=/path/to/local/sweep/root \
CATALOG_REGISTRY_PATH=/tmp/catalog/registry \
CATALOG_TTL_AUDIT_PATH=/tmp/catalog/ttl_audit \
cargo run -p catalog-api
```

## Configuration

All configuration is env-var driven (see `catalog-api/src/config.rs` for the
authoritative source). Defaults suit local dev / tests.

| Env var | Default | Description |
|---|---|---|
| `CATALOG_BIND_ADDR` | `0.0.0.0:8080` | Bind address for the public REST API. |
| `CATALOG_METRICS_BIND_ADDR` | `0.0.0.0:9090` | Bind address for the internal `/metrics` + `/healthz` server (network-policy-restricted; the main API router has no `/metrics` route). |
| `CATALOG_LEADER_MODE` | `kube` | `kube` (real `coordination.k8s.io/v1` Lease), `forced-on` (always leader, no k8s API — local dev/tests), or `forced-off` (always non-leader). |
| `CATALOG_LEASE_NAMESPACE` | `default` | k8s namespace for the Lease object (`kube` mode only). |
| `CATALOG_LEASE_NAME` | `catalog-api-leader` | Lease object name (`kube` mode only). |
| `CATALOG_POD_NAME` | `catalog-api-<pid>` | Holder identity advertised in the Lease (`kube` mode only; typically the downward-API pod name). |
| `CATALOG_LEADER_TICK_INTERVAL_SECS` | `10` | How often the leader-election task re-ticks (acquire/renew attempt). |
| `CATALOG_LEASE_DURATION_SECS` | `30` | Lease duration advertised to the k8s API (`kube` mode only). |
| `CATALOG_REGISTRY_PATH` | `_catalog/registry` | URI (or relative path) of the `_catalog/registry` Lance dataset. |
| `CATALOG_TTL_AUDIT_PATH` | `_catalog/ttl_audit` | URI (or relative path) of the `_catalog/ttl_audit` Lance dataset TTL `apply` appends to. |
| `CATALOG_REGISTRY_REFRESH_INTERVAL_SECS` | `5` | How often every pod re-reads the registry into its in-memory cache. |
| `CATALOG_SWEEP_ROOT_URI` | `s3://onroad-perception-datasets/scenario_dataset_export` | URI of the sweep root (S3 or local filesystem path for dev). |
| `CATALOG_SWEEP_INTERVAL_SECS` | `1800` | How often the leader runs a full sweep pass (~30 min default). |

For the local MinIO overlay, S3 endpoint/credentials are supplied via the
standard `AWS_*` env vars (`AWS_ENDPOINT`, `AWS_ALLOW_HTTP`,
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`, etc.) consumed by
`object_store::parse_url_opts` and Lance's `AwsStoreProvider`. See
`deploy/overlays/local/configmap-patch.yaml`.

## REST API surface

The public router (`/v1` + `/ext/v1`) is wired in `catalog-api/src/api.rs`.
Read endpoints serve from the in-memory registry cache on every pod; mutation
endpoints are leader-only (503 on a non-leader pod — clients retry; no
cross-pod forwarding in v1.0.0).

- `GET /healthz` — liveness (on the internal port).
- `GET /v1/namespaces` — list distinct namespaces (derived from registered tables).
- `GET /v1/namespaces/:id` — describe a namespace (id is `.`-joined path).
- `GET /v1/tables` — list table ids.
- `GET /v1/table/:id` — describe a table (full `TableEntry`).
- `PUT /v1/table/:id` — declare/update a table (set `owner`/`ttl_policy`; idempotent; leader-only).
- `DELETE /v1/table/:id` — deregister a table (leader-only; the next sweep re-discovers a table that still exists on S3).
- `GET /ext/v1/tables?expand=...` — enriched listing (always returns full detail, no pagination in v1.0.0).
- `GET /ext/v1/tables/:id/versions/:vid` — single version detail.
- `PUT /ext/v1/tables/:id/versions/:vid/protect` — set the `protected` flag on a version (TTL-exempt; leader-only).
- `GET /ext/v1/tables/:id/ttl/dryrun` — list TTL-eligible versions + reclaimable bytes (read-only, any pod).
- `POST /ext/v1/tables/:id/ttl/apply` — hard-delete eligible versions (leader-only, irreversible, audited).
- `GET /ext/v1/tables/:id/ttl/audit` — read the TTL audit log for a table (read-only, any pod).
- `GET /debug/registry`, `GET /debug/is_leader` — internal debug endpoints (read-only, unauthenticated; revisit gating before exposing beyond an admin boundary).

`GET /metrics` is served on the separate internal port (default 9090) by the
internal server, not on the main API router.

## Monitoring

`deploy/base/monitoring/` ships a standalone Prometheus + Grafana, a
ServiceMonitor selecting the catalog-api `metrics` port, alert rules loaded by
the standalone Prometheus from a ConfigMap (`NoLeader`, `FreshnessBreach`,
`HydrationNotReady`, `SweepStalled`, `S3Throttling`, `TTLDeletesFailed`,
`ReadAvailabilityBurn`, `ReadLatencyHigh`), and Grafana dashboard ConfigMaps
(Overview, Sweep & Convergence, TTL & Storage). The
metric names and label cardinality contract are documented in
`catalog-api/src/metrics.rs`.

## Known limitations

v1.0.0 ships the read + control plane (sweep, registry, REST API, TTL engine,
monitoring, static frontend). Items deferred beyond v1.0.0 are listed in
[`ARCHITECTURE.md` ("Known limitations / v1.0.0 scope")](ARCHITECTURE.md#known-limitations--v100-scope)
and in the runbooks (real-cluster validation steps that need a live kind
cluster / real browser).
