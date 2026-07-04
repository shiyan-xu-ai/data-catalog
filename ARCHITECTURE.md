# Architecture (v1.0.0, as-built)

This document describes the v1.0.0 system as implemented. The broader
long-term vision lives in [`docs/design.md`](docs/design.md); v1.0.0 is a
subset (see [Known limitations / v1.0.0 scope](#known-limitations--v100-scope)
at the end). Where v1.0.0 concretizes or deviates from the design, the
deviations are recorded in [`docs/design.md`](docs/design.md) under
"v1.0.0 as-built" notes.

## Overview

One binary, `catalog-api`, serves reads, runs the periodic S3 sweep, and runs
the TTL engine. Leadership is determined by a k8s Lease so only one pod sweeps
and mutates the registry at a time; non-leader pods serve reads from an
in-memory registry cache they periodically refresh. A static SPA on S3 +
CloudFront is the operator UI. Prometheus + Grafana ship in-repo for
monitoring.

```
                     ┌──────────────────────────────────────────────┐
                     │  catalog-api (leader pod)                    │
   S3 sweep root ───▶│  sweep loop ──▶ apply_sweep_result ──▶ _catalog/registry (Lance)  │
   (timestamp-       │  TTL engine ──▶ hard delete ──▶ _catalog/ttl_audit (Lance)        │
    path versions)   │  REST API ──▶ in-memory registry cache ◀─── refresh tick          │
                     │  leader election: k8s Lease (CAS acquire/renew)                  │
                     └──────────────────────────────────────────────┘
                            │                          │
                  read replicas (cache)          Prometheus /metrics (:9090)
                            │                          │
                     static SPA (S3+CloudFront)   Grafana dashboards
```

## The sweep

`catalog-store/src/sweep.rs` implements the periodic sweep of the configured
root (`CATALOG_SWEEP_ROOT_URI`, default
`s3://onroad-perception-datasets/scenario_dataset_export`).

### Discovery

The sweep lists the root for table directories, then for each table lists
timestamp directories. A timestamp directory name is parsed as
`YYYY-MM-DD_HH-MM-SS` or `YYYY-MM-DD-HH-MM-SS` (name variance is handled),
normalized to an ISO8601 version id. One timestamp directory = one catalog
version. The full history is supported: every timestamp version from earliest
to latest, regardless of pre/post cutoff layout.

### Shape classification

Each timestamp directory is classified into one of five shapes:

- `full` — a main lance directory (openable as a Lance dataset) + `segments/`
  [+ `dataset.sidecar/`].
- `lance_only` — main lance present, no `segments/`.
- `seg_only` — `segments/` only, no main lance (partial; cannot open, aux only).
- `lance_only_partial` — the apparent main lance directory contains only
  nested lances (`index_datasets/`, `tag_datasets/`) with no
  `_versions/`+`_transactions/` at its own root (partial).
- `empty` — no recognized directories.

A `partial` flag is set on any version whose shape is not `full`.

### Main lance detection

The main lance directory is detected by the presence of `_versions/` and
`_transactions/` at the directory root, never by name. The name varies in
practice (`dataset.lance/` for most tables, `dataset/` for some). If a
candidate directory lacks `_versions/`+`_transactions/` at its root and
contains only nested lances, the version is `lance_only_partial`.

### Size split

Storage bytes are attributed per-component so that pre-cutoff versions (where
sidecar directories live inside `dataset.lance/` and inflate its size) are
correctly decomposed:

- `lance_core_bytes` — `_versions/`, `_transactions/`, `_indices/`, `data/`.
- `sidecar_bytes` — sidecar directories, wherever located (top-level
  `dataset.sidecar/` if present, else the known sidecar subdirs inside the
  main lance dir).
- `segments_bytes` — the `segments/` directory.
- `other_aux_bytes` — any other top-level aux directory not accounted for
  above.

`storage_bytes_total` is the logical (deduped) sum of these components for the
version. See the [TTL runbook](docs/RUNBOOK-TTL.md) for the
logical-vs-physical distinction that matters for `reclaimable_bytes`.

### Aux format detection

Aux directories (`segments/`, `dataset.sidecar/`, `curated_csv/`,
`row_counts/`, nested lances, etc.) are recorded as `AuxEntry`s with a detected
`format`:

- `parquet` — `_SUCCESS` + `part-*.parquet`.
- `lance` — `_versions/` + `_transactions/`.
- `csv` — `*.csv`.
- `mixed` / `unknown` — otherwise, with a fingerprint via LIST.

Aux names are not hardcoded beyond the sidecar/segments/lance-core sets; the
dir name is recorded as the `role` (no ontology layer in v1.0.0).

### Pre+post 2026-06-26 cutoff aux layout

A layout transition occurred around 2026-06-26:

- **Pre-cutoff** (< 2026-06-26): sidecar directories live *inside*
  `dataset.lance/` (alongside lance-core directories). The sweep reads them
  from there.
- **Post-cutoff** (>= 2026-06-26): sidecar directories move to a top-level
  `dataset.sidecar/`; `dataset.lance/` is clean (lance-core only).
- **Transition** (~2026-06-12): sidecar content is duplicated both inside
  `dataset.lance/` and at top-level `dataset.sidecar/` (dual-write). The sweep
  deduplicates: when a top-level `dataset.sidecar/` is present it is preferred;
  otherwise sidecar bytes are read from inside the main lance dir.

The sweep supports both layouts so the full version history is cataloged
correctly. The real-world layout findings are captured in
[`docs/design.md`](docs/design.md) ("v1.0.0 as-built" notes).

## The registry

The registry is a Lance dataset under `_catalog/registry` (path configurable
via `CATALOG_REGISTRY_PATH`). It stores one `TableEntry` per table: id, name,
namespace, root_location, owner, ttl_policy, last_swept, versions[], and
aux_latest[]. Each `TableVersion` carries version_id, timestamp,
snapshot_path, shape, partial, protected, the per-component byte totals,
row_count, num_fragments, schema_json, num_indices, aux[], and swept_at.

### Single-writer guarantee

Only the leader pod writes the registry. Leadership is determined by a k8s
`coordination.k8s.io/v1` Lease (`catalog-api/src/leader.rs`):

- **Acquire** (lease absent or expired): a `replace` (PUT) carrying the
  observed `resourceVersion` — a CAS guarded by the API server's 409-on-stale
  `resourceVersion` semantics. Two candidates racing on an expired lease
  cannot both win: the second's PUT is rejected with 409 and the loser steps
  down. (Server-side apply `Patch::Apply` is never used for this branch — its
  same-manager writes never conflict, which was the original split-brain bug.)
- **Renew** (we hold a live, non-expired lease): also CAS-guarded via
  `replace` + `resourceVersion`. Expiry is checked *before* ownership in
  `decide()`, so a lapsed former holder whose own lease has expired routes
  through `Acquire`, not `Renew`.
- **BackOff** (someone else holds a live lease): report non-leader.
- **Create** (lease absent, 404 on GET): an atomic `create`; a lost create
  race yields 409 and the loser reports non-leader.

A shared `RegistryWriteLock` (`catalog-api/src/registry_lock.rs`,
`Arc<tokio::sync::Mutex<()>>`) serializes the read-modify-write critical
section of every registry writer — the sweep loop and all four API mutation
handlers (`DeclareTable`, `DeregisterTable`, version `protect`, TTL `apply`)
— on the leader pod, so no two writers interleave.

### Registry cache

Every pod (leader or not) maintains an in-memory `RegistryCache`
(`Arc<RwLock<Vec<TableEntry>>>`) refreshed every
`CATALOG_REGISTRY_REFRESH_INTERVAL_SECS` (default 5s). All read endpoints
serve from this cache. A leader's mutation writes the registry then
immediately re-populates the cache in-process, so a subsequent read on the
same pod sees the change without waiting for the next refresh tick.

## REST API surface

See [README.md](README.md#rest-api-surface) for the route list. The
authoritative source is `catalog-api/src/api.rs::api_router`.

- **Basic ops** (`/v1`): `ListNamespaces`, `DescribeNamespace`, `ListTables`,
  `DescribeTable`, `DeclareTable` (PUT, sets owner/ttl_policy, idempotent),
  `DeregisterTable` (DELETE). Namespaces are a derived view, not stored state.
- **`/ext` enriched surface**: `GET /ext/v1/tables` (always full detail, no
  pagination in v1.0.0), `GET /ext/v1/tables/:id/versions/:vid`, and the TTL
  endpoints (`dryrun`, `apply`, `audit`) + version `protect`.
- Mutations are leader-only (503 on a non-leader pod; no cross-pod forwarding
  in v1.0.0).

## TTL engine

`catalog-core/src/ttl.rs` (eligibility) + `catalog-api/src/api.rs` (endpoints).
The TTL engine hard-deletes old timestamp version directories from S3 under a
per-table API policy. Because the delete is irreversible, the engine is
gated by a mandatory dry-run-before-apply workflow, a `protected` flag, and a
shape safety gate. Full operational guidance is in
[`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md).

### Eligibility (`ttl_eligible_versions`)

A version is eligible for deletion only if *all* of the following hold:

- it is not `protected` (never eligible, regardless of policy or shape);
- its shape passes the safety gate (`full`, `lance_only`, or `seg_only` —
  `lance_only_partial` and `empty` are refused);
- it fails *every* set threshold on the policy:
  - `keep_last_n` (if set): the version is not in the top-N most recent by
    `timestamp`.
  - `max_age_days` (if set): `now - timestamp` exceeds the threshold.
- If *neither* threshold is set (no policy), nothing is ever eligible (safe
  default). The AND-logic means a version is deleted only if it exceeds
  *both* thresholds when both are set (within either threshold => kept).

### Dry-run, apply, audit

- `GET /ext/v1/tables/:id/ttl/dryrun` — read-only, any pod; returns candidate
  version ids + `reclaimable_bytes` (the sum of `storage_bytes_total` over
  candidates; logical/deduped, not physical footprint).
- `POST /ext/v1/tables/:id/ttl/apply` — leader-only, irreversible. Recomputes
  eligibility fresh (never trusts a stale dry-run response). For each eligible
  version: physically deletes its entire `<table>/<timestamp>/` prefix tree,
  appends a `TtlAuditRecord` to `_catalog/ttl_audit`, and removes the version
  from the registry — all inside the shared `write_lock`-guarded critical
  section. Idempotent: re-apply is a no-op for already-removed versions.
- `GET /ext/v1/tables/:id/ttl/audit` — read-only, any pod; returns the
  filtered audit records for the table (empty list if no apply has ever run
  or the audit dataset does not exist yet).
- `PUT /ext/v1/tables/:id/versions/:vid/protect` — sets the `protected` flag
  (TTL-exempt; leader-only).

## Monitoring

`catalog-api/src/metrics.rs` instruments the service with the `metrics`
facade + `metrics-exporter-prometheus`. All metric names carry the `catalog_`
prefix. Label cardinality is bounded: `table_id` and `version_id` are never
label values; the HTTP RED metrics use the matched route *pattern*
(e.g. `/v1/table/:id`, not the concrete path) as the `route` label, interned
once per distinct pattern. Metric names shipped in v1.0.0:

- HTTP RED: `catalog_http_requests_total`, `catalog_http_request_duration_seconds`
- Sweep: `catalog_sweep_tables_checked_total`, `catalog_sweep_cycle_duration_seconds`,
  `catalog_snapshot_staleness_seconds`
- Freshness: `catalog_hydration_ready`
- Leader: `catalog_is_leader`
- TTL: `catalog_ttl_deletes_total`, `catalog_ttl_apply_total`, `catalog_ttl_reclaimable_bytes`
- Object store: `catalog_s3_operations_total`
- Runtime: `tokio_*` (tokio-metrics), `process_*` (metrics-process)

`deploy/base/monitoring/` ships Prometheus + Grafana deployments, a
ServiceMonitor selecting the `metrics` port, a PrometheusRule
(`NoLeader`, `FreshnessBreach`, `SweepStalled`, `S3Throttling`,
`TTLDeletesFailed`, `ReadAvailabilityBurn`, `ReadLatencyHigh`), and Grafana
dashboard ConfigMaps (Overview, Sweep & Convergence, TTL & Storage).

## Frontend

`frontend/` is a vanilla TS + Vite static SPA (hash-based routing, no
framework). The list view reads `/ext/v1/tables`; the drill-down reads
`/v1/table/:id`; the TTL view calls dry-run + apply (with a
type-the-table-name confirmation gate before the irreversible POST) and the
audit endpoint. Deploy is to S3 + CloudFront; see
[`frontend/docs/DEPLOY.md`](frontend/docs/DEPLOY.md) and the
[deploy runbook](docs/RUNBOOK-DEPLOY.md#frontend).

## Known limitations / v1.0.0 scope

v1.0.0 ships the read + control plane (sweep, registry, REST API, TTL engine,
monitoring, static frontend). The following are documented deferrals, not
in-sandbox gaps:

- **Real-cluster validation not performed in-sandbox.** The KubeLeaseElector
  end-to-end kill-leader procedure, the kind `tilt up` boot, the real-S3/MinIO
  TTL delete conformance run, and the load smoke against the real 65-table
  sweep root all require a live kind cluster / real S3 and are documented as
  procedures in [`docs/smoke-test.md`](docs/smoke-test.md) and the
  [deploy runbook](docs/RUNBOOK-DEPLOY.md#leader-election-validation). The
  unit tests prove the client-side CAS logic against a fake backend
  implementing the documented resourceVersion-conflict contract.
- **Registry write has no storage-layer CAS backstop.** The single-writer
  guarantee rests on the atomic lease acquire/renew (CAS-guarded) + the shared
  in-process `RegistryWriteLock`. The remaining unguarded surface is the
  cross-pod leadership-flip window (bounded, self-correcting); a full
  fencing-token / Lance CAS commit protocol is out of scope for v1.0.0.
- **TTL audit-vs-registry write is not atomic across the two Lance tables.**
  See [`docs/RUNBOOK-TTL.md`](docs/RUNBOOK-TTL.md) (audit log is best-effort
  under a mid-apply crash).
- **`/debug/*` routes and mutation routes are unauthenticated.** Acceptable
  while the service is behind an admin boundary; revisit auth/gating before
  exposing beyond that.
- **CORS is permissive.** Tighten to the deployed frontend origin once the
  S3/CloudFront URL exists.
- **`/ext/v1/tables` has no pagination.** Fine at v1.0.0's admin scale
  (dozens of tables); revisit if the registry grows large.
- **Design features dropped for v1.0.0:** lineage, MCP, Flight SQL,
  profiling, maintenance/compaction engine, cost analysis, credential
  vending, operation history, inbox. See [`docs/design.md`](docs/design.md).
