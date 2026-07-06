# Release notes — v1.0.0

First release of the Lance data catalog service.

## Features

- **S3 sweep with full-history timestamp-path versioning.** Periodically
  sweeps a configured S3 root, discovers Lance tables under timestamp-named
  snapshot directories (`<table>/<YYYY-MM-DD_HH-MM-SS>/`), and records
  per-version metadata into a Lance-backed registry under `_catalog/`.
  Supports both pre- and post-2026-06-26 cutoff aux layouts (sidecar inside
  `dataset.lance/` vs. top-level `dataset.sidecar/`) so the full version
  history is cataloged.
- **Per-version shape classification + size split.** Each version is
  classified `full` / `lance_only` / `seg_only` / `lance_only_partial` /
  `empty`, with a `partial` flag. Storage bytes are split per-component
  (`lance_core`, `sidecar`, `segments`, `other_aux`) so pre-cutoff versions
  attribute sidecar bytes correctly.
- **Aux format detection.** Aux directories are recorded with a detected
  format (`parquet`, `lance`, `csv`, `mixed`, `unknown`) and a fingerprint.
- **Leader-elected single-writer registry.** A k8s `coordination.k8s.io/v1`
  Lease with CAS-guarded acquire + renew (no server-side apply) ensures only
  one pod writes the registry. A shared in-process `RegistryWriteLock`
  serializes all registry writers (sweep loop + API mutations). Read replicas
  serve from an in-memory registry cache refreshed every 5s.
- **REST API.** Basic ops (`/v1`: ListNamespaces, DescribeNamespace,
  ListTables, DescribeTable, DeclareTable, DeregisterTable) + an enriched
  `/ext/v1` surface (full-detail listing, version detail, version `protect`).
  Read endpoints serve from the cache on every pod; mutations are leader-only.
- **TTL engine (hard delete).** Per-table API policy
  (`keep_last_n` + `max_age_days`, AND-logic), `protected` flag exemption,
  shape safety gate, mandatory dry-run-before-apply workflow, irreversible
  hard delete, and an audited apply path. Audit log readable via
  `GET /ext/v1/tables/:id/ttl/audit`.
- **Prometheus + Grafana monitoring.** In-repo manifests: ServiceMonitor,
  PrometheusRule (`NoLeader`, `FreshnessBreach`, `SweepStalled`,
  `S3Throttling`, `TTLDeletesFailed`, `ReadAvailabilityBurn`,
  `ReadLatencyHigh`), and Grafana dashboards (Overview, Sweep & Convergence,
  TTL & Storage). Bounded-label HTTP RED metrics using the matched route
  pattern (not the concrete path).
- **kustomize base + overlays.** `deploy/base/` (Deployment, Service, RBAC,
  ConfigMap, IRSA stub) + `local` (kind + MinIO + fixtures), `staging`, and
  `prod` (IRSA, HPA, PDB, anti-affinity) overlays. Tiltfile for local dev.
- **Minimal static frontend.** Vanilla TS + Vite SPA (hash-based routing, no
  framework) on S3 + CloudFront: table list, drill-down, TTL dry-run/apply
  (with type-the-table-name confirmation gate) + audit log view.

## Configuration

Env-var driven (see `catalog-api/src/config.rs`). Defaults suit local dev.
Key vars: `CATALOG_SWEEP_ROOT_URI`, `CATALOG_REGISTRY_PATH`,
`CATALOG_LEADER_MODE` (`kube` / `forced-on` / `forced-off`),
`CATALOG_SWEEP_INTERVAL_SECS`, `CATALOG_LEASE_DURATION_SECS`.

## Documentation

- [`README.md`](../README.md) — overview, quickstart, config table, API surface.
- [`ARCHITECTURE.md`](../ARCHITECTURE.md) — as-built architecture.
- [`docs/RUNBOOK-DEPLOY.md`](RUNBOOK-DEPLOY.md) — deploy (local/staging/prod),
  leader-election validation, frontend deploy.
- [`docs/RUNBOOK-TTL.md`](RUNBOOK-TTL.md) — TTL policy semantics, dry-run/
  apply/audit workflow, `protected` flag, known caveats.
- [`docs/smoke-test.md`](smoke-test.md) — kind smoke-test procedure.
- [`docs/design.md`](design.md) — long-term design (v1.0.0 as-built notes
  appended).

## Known limitations

- Real-cluster validation (KubeLeaseElector kill-leader e2e, kind boot,
  real-S3/MinIO TTL conformance, load smoke against the real 65-table root)
  requires a live cluster — procedures documented in the runbooks.
- Registry write has no storage-layer CAS backstop; single-writer guarantee
  rests on the atomic lease acquire/renew + the in-process write lock.
- TTL audit-vs-registry write is not atomic across the two Lance tables
  (audit log is best-effort under a mid-apply crash).
- `/debug/*` and mutation routes are unauthenticated; CORS is permissive.
  Acceptable behind an admin boundary; revisit before exposing beyond that.
- `/ext/v1/tables` has no pagination (fine at admin scale).
- Design features dropped for v1.0.0: lineage, MCP, Flight SQL, profiling,
  maintenance/compaction engine, cost analysis, credential vending, operation
  history, inbox. See [`docs/design.md`](design.md).

## Since v1.0.0

- **S3-only deployment on Apps Platform (Cloud Run).** Leader election
  (k8s Lease), the in-process write lock, and the Prometheus/Grafana/
  kustomize stack described above are gone. Catalog state now splits into a
  derived snapshot (sweep output, last-wins, no coordination) and a small
  per-table authored overlay (`owner`/`ttl_policy`/`protected`) guarded by
  S3 ETag-CAS. Sweep runs from a Cloud Scheduler-triggered endpoint instead
  of a background loop. Observability moved to Cloud Logging; there is no
  `/metrics` Prometheus endpoint. See `ARCHITECTURE.md` and the design
  doc's as-built notes for the current model.
- **Sweep performance.** A single per-version LIST now drives shape/size/aux
  classification, all registered tables' pending versions sweep
  concurrently from one bounded work queue, and clean immutable versions
  already in the prior snapshot are carried forward without re-opening
  their datasets. A `CATALOG_SWEEP_DEEP_STATS` knob (`all`/`latest`/`none`)
  gates the index-load + `count_rows` fallback per version.
- **Richer auxiliary table metadata.** Aux entries record a `category`
  (`sidecar` vs `nested_sidecar` — a lance dataset discovered nested inside
  another table's `dataset.lance/`), row count, schema, and Lance/writer
  version, derived from the dataset manifest at no extra S3 cost.
  `GET /ext/v1/tables/:id/versions/:vid/aux/sample` reads sample rows from
  an aux table (Lance or Parquet, via DataFusion).
- **Frontend rewritten** on React + TanStack Router/Query + shadcn/ui (see
  [`docs/FRONTEND.md`](FRONTEND.md)), replacing the vanilla-TS/Vite SPA.
