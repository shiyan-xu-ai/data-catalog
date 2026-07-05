# Data Catalog

A Lance data catalog service: an S3-native metadata catalog for Lance tables,
written in Rust (axum, the `lance` crate, `object_store`). It runs as a single
stateless container on Applied's Apps Platform (Cloud Run).

The service sweeps an S3 root, discovers timestamp-path versioned Lance tables,
and records per-version metadata (shape, storage size split by component, row
count, schema, aux entries) into a Lance-backed registry under `_catalog/`. A
REST API exposes the catalog (basic ops + an enriched `/ext` listing) and a
per-table TTL engine that hard-deletes old versions under an explicit API
policy. A minimal static SPA (vanilla TS + Vite) is served from the same origin
as the API for browsing and TTL operations.

The full long-term design lives in [`docs/design.md`](docs/design.md). The
as-built architecture is documented in [`ARCHITECTURE.md`](ARCHITECTURE.md).

## State model: S3 is the sole store

There is no database and no cross-instance coordinator (no leader election, no
lock). All catalog state lives on S3, split by how it is produced:

- **Derived state** — versions, shapes, byte splits, row counts, schemas, aux.
  A pure function of immutable S3 content, so any full sweep can regenerate it.
  The sweep writes the whole `_catalog/registry` Lance dataset **last-wins**,
  with no coordination: concurrent or double-fired sweeps are safe.
- **Authored state** — per-table `owner` + `ttl_policy` and per-version
  `protected`. The only human-mutated data, stored as one small JSON object per
  table at `_catalog/meta/<table_id>.json` and updated with object-store
  conditional writes (ETag compare-and-set). A per-table object is its own
  conflict domain, so S3 itself serializes concurrent edits — no app lock.

A read merges the two back into the `TableEntry` wire shape the REST API and
frontend expect. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the full model.

## Workspace layout

Cargo workspace with three crates:

- `catalog-core` — shared types (`TableEntry`, `TableVersion`, `TtlPolicy`,
  `TtlAuditRecord`, `AuxEntry`, `Namespace`, `VersionShape`, `AuxFormat`), the
  Lance registry reader/writer, the sweep-result reconcile, and the TTL
  eligibility computation.
- `catalog-store` — object-store IO, the S3 sweep (discovery, timestamp-path
  version classification, shape detection, per-component size split, aux format
  detection, pre+post cutoff layout handling), the TTL hard-delete helper, and
  the authored-overlay ETag-CAS store (`MetaStore`).
- `catalog-api` — the `catalog-api` binary: the axum HTTP server, the merged
  read model, the request-triggered sweep, and the TTL engine.

Additional directories:

- `frontend/` — vanilla TS + Vite static SPA (table list, drill-down, TTL
  dry-run/apply/audit). Built into the container and served same-origin.
- `docs/` — design doc, deploy + TTL runbooks.

## Quickstart (run the API standalone)

The binary runs directly against a local filesystem sweep root with an
in-memory authored overlay — no S3, no cluster:

```sh
# Build the SPA once so the server can serve it (optional; API works without it).
cd frontend && bun install && bun run build && cd ..

CATALOG_SWEEP_ROOT_URI=/path/to/local/sweep/root \
CATALOG_REGISTRY_PATH=/tmp/catalog/registry.lance \
CATALOG_TTL_AUDIT_PATH=/tmp/catalog/ttl_audit.lance \
CATALOG_META_BASE_URI=memory \
CATALOG_WEBUI_DIR=frontend/dist \
cargo run -p catalog-api
# then, in another shell:
curl -s -X POST localhost:8080/internal/jobs/sweep   # run one sweep
curl -s localhost:8080/v1/tables                     # list discovered tables
open http://localhost:8080/                          # the SPA
```

`CATALOG_META_BASE_URI=memory` uses a non-persistent in-memory overlay, which is
fine for a quick local run. Persisting authored state needs an S3/MinIO overlay
(`LocalFileSystem` does not implement the conditional writes the overlay uses).

To run the frontend with hot-reload against a running API:

```sh
cd frontend && bun run dev
# open http://localhost:5173 — the Vite dev proxy forwards /v1 and /ext to :8080
```

## Configuration

All configuration is env-var driven (see `catalog-api/src/config.rs` for the
authoritative source). Defaults suit local dev / tests.

| Env var | Default | Description |
|---|---|---|
| `PORT` | — | Injected by Cloud Run; when set, the server binds `0.0.0.0:$PORT`. |
| `CATALOG_BIND_ADDR` | `0.0.0.0:8080` | Bind address when `PORT` is unset. |
| `CATALOG_REGISTRY_PATH` | `_catalog/registry` | URI (or path) of the derived-snapshot Lance dataset. |
| `CATALOG_TTL_AUDIT_PATH` | `_catalog/ttl_audit` | URI (or path) of the TTL audit Lance dataset. |
| `CATALOG_META_BASE_URI` | `memory` | Base of the authored overlay objects: `memory` (in-memory, non-persistent) or `s3://bucket/prefix` (S3/MinIO). A plain filesystem path is rejected — the overlay needs conditional writes. |
| `CATALOG_SWEEP_ROOT_URI` | `s3://onroad-perception-datasets/scenario_dataset_export` | Sweep root (S3 URI or local filesystem path). |
| `CATALOG_CACHE_TTL_SECS` | `5` | How long the merged read view is served before it revalidates against storage. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Directory of the built SPA to serve at `/`. Absent → API-only. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager name prefix for AWS credentials (Cloud Run sets `K_SERVICE` to the service name). |

AWS S3 credentials: on Cloud Run they are read from Secret Manager at startup
(there is no ambient AWS credential — see [`ARCHITECTURE.md`](ARCHITECTURE.md)
and the [deploy runbook](docs/RUNBOOK-DEPLOY.md)). Locally, the standard `AWS_*`
env vars (`AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
`AWS_ALLOW_HTTP`, `AWS_VIRTUAL_HOSTED_STYLE_REQUEST`, `AWS_DEFAULT_REGION`) are
consumed by `object_store::parse_url_opts`, so an `AWS_ENDPOINT_URL` pointed at
MinIO exercises the real conditional-write path.

## REST API surface

The public router (`/v1` + `/ext/v1`) is wired in `catalog-api/src/api.rs`.
Reads serve the merged view (derived snapshot + authored overlay), lazily
revalidated. Any instance can serve any request — reads and mutations alike —
because the snapshot is written last-wins and the overlay is serialized per
table by S3's conditional writes.

- `GET /healthz` — liveness (`ok` from boot).
- `GET /readyz` — readiness; `503` until the first successful view load, then `200`.
- `GET /v1/namespaces` — list distinct namespaces (derived from cataloged tables).
- `GET /v1/namespaces/:id` — describe a namespace (id is `.`-joined path).
- `GET /v1/tables` — list table ids.
- `GET /v1/table/:id` — describe a table (full `TableEntry`).
- `PUT /v1/table/:id` — declare/update a table (set `owner`/`ttl_policy`; idempotent).
- `DELETE /v1/table/:id` — deregister (clears the authored overlay; a table still present on S3 stays cataloged via the derived snapshot).
- `GET /ext/v1/tables?expand=...` — enriched listing (full detail; no pagination).
- `GET /ext/v1/tables/:id/versions/:vid` — single version detail.
- `PUT /ext/v1/tables/:id/versions/:vid/protect` — set/clear a version's TTL-exempt `protected` flag.
- `GET /ext/v1/tables/:id/ttl/dryrun` — list TTL-eligible versions + reclaimable bytes (read-only).
- `POST /ext/v1/tables/:id/ttl/apply` — hard-delete eligible versions (irreversible, audited).
- `GET /ext/v1/tables/:id/ttl/audit` — read a table's TTL audit log.
- `POST /internal/jobs/sweep` — run one sweep pass. Triggered by Cloud Scheduler; idempotent under retry/double-fire.

## Observability

Structured JSON logs (sweep summaries, per-table sweep failures, TTL outcomes)
go to stdout for Cloud Logging. HTTP request/latency (RED) metrics come from
Cloud Run's built-in Cloud Monitoring; there is no Prometheus endpoint.
`RUST_LOG` tunes log verbosity (default `info`, with Lance's per-operation
chatter quieted to `warn`).

## Deployment

Deployed to Cloud Run via Apps Platform (`apps-platform app deploy --local`),
with the sweep driven by Cloud Scheduler and AWS credentials in Secret Manager.
See [`docs/RUNBOOK-DEPLOY.md`](docs/RUNBOOK-DEPLOY.md).
