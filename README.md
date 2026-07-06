# Data Catalog

A Lance data catalog service: an S3-native metadata catalog for Lance tables,
written in Rust (axum, the `lance` crate, `object_store`). It runs as a single
stateless container on Applied's Apps Platform (Cloud Run).

The service is regional: one deployment per region, colocated with that
region's target buckets, declared in a committed [`catalog-config.yaml`](catalog-config.yaml)
(region, buckets, each bucket's registered namespaces, admins — see
[Catalog config](#catalog-config-catalog-configyaml)). The catalog is a
curated allowlist within that scope: an operator registers the tables to track
(`PUT /v1/table/:id`, id `<region>:<bucket>:<namespace>:<name>`), and the sync
records per-version metadata (shape, storage size split by component, row
count, schema, aux entries) for exactly those registered tables — from
timestamp-path versioned Lance tables under each registered namespace — into a
Lance-backed registry under `_catalog/`. Unregistered tables on S3 are ignored.
A REST API exposes the catalog (basic ops + an enriched `/ext` listing,
storage analysis, and user/identity endpoints) and a per-table TTL engine. A
static SPA is served from the same origin as the API for browsing.

The full long-term design lives in [`docs/design.md`](docs/design.md). The
as-built architecture is documented in [`ARCHITECTURE.md`](ARCHITECTURE.md).

## State model: S3 is the sole store

There is no database and no cross-instance coordinator (no leader election, no
lock). All catalog state lives on S3, split by how it is produced:

- **Derived state** — versions, shapes, byte splits, row counts, schemas, aux.
  A pure function of immutable S3 content, so any sync can regenerate it. The
  sync re-derives the registered tables and writes the whole `_catalog/registry`
  Lance dataset **last-wins**, with no coordination: concurrent or double-fired
  syncs are safe.
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
  `TtlAuditRecord`, `AuxEntry`, `Namespace`, `VersionShape`, `AuxFormat`,
  `StoragePrefixStat`, `UserRecord`), the composite table-id parser/composer,
  the Lance registry/storage-scan/users readers and writers, `aux_latest`
  derivation, and the TTL eligibility computation.
- `catalog-store` — object-store IO, the S3 sync (discovery, timestamp-path
  version classification, shape detection, per-component size split, aux format
  detection, pre+post cutoff layout handling), the TTL hard-delete helper, and
  the authored-overlay ETag-CAS store (`MetaStore`).
- `catalog-api` — the `catalog-api` binary: the axum HTTP server, the deployment
  catalog config loader, the merged read model, the multi-bucket
  request-triggered sync (with its storage-analysis tail), the IAP identity
  middleware, and the TTL engine.

Additional directories:

- `frontend/` — vanilla TS + Vite static SPA (table list, drill-down, TTL
  dry-run/apply/audit). Built into the container and served same-origin.
- `docs/` — design doc, deploy + TTL runbooks.

## Quickstart (run the API standalone)

The binary is regional: at startup it loads `catalog-config.yaml` (path
overridable via `CATALOG_CONFIG_PATH`) to learn its region, target buckets,
and each bucket's registered namespaces, then builds one S3 object store per
bucket — so a local run needs an S3-compatible endpoint (a MinIO container,
or a real/OCI bucket) rather than a bare filesystem sync root. The authored
overlay can still be a non-persistent in-memory store for a quick run:

```sh
# Build the SPA once so the server can serve it (optional; API works without it).
# `--bun` runs the toolchain under Bun's runtime (Node 18 crashes on an upstream
# unplugin `import.meta.dirname` use; Bun handles it).
cd frontend && bun install && bun --bun run build && cd ..

# catalog-config.yaml (repo root) must declare a bucket/namespace this endpoint
# can serve; point AWS_ENDPOINT_URL at MinIO (or a real bucket's S3-compat API).
export AWS_ENDPOINT_URL=http://localhost:9000
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_ALLOW_HTTP=true
export AWS_VIRTUAL_HOSTED_STYLE_REQUEST=false

CATALOG_REGISTRY_PATH=/tmp/catalog/registry.lance \
CATALOG_TTL_AUDIT_PATH=/tmp/catalog/ttl_audit.lance \
CATALOG_STORAGE_SCAN_PATH=/tmp/catalog/storage_scan.lance \
CATALOG_USERS_PATH=/tmp/catalog/users.lance \
CATALOG_META_BASE_URI=memory \
CATALOG_WEBUI_DIR=frontend/dist \
cargo run -p catalog-api
# then, in another shell — register a table (composite id: region:bucket:namespace:name),
# then sync it:
curl -s -X PUT localhost:8080/v1/table/<region>:<bucket>:<namespace>:<table-dir-name>
curl -s -X POST localhost:8080/internal/jobs/sync        # sync the registered tables
curl -s localhost:8080/v1/tables                          # list registered tables
open http://localhost:8080/                               # the SPA
```

Nothing is synced until a table is registered — the sync only processes
registered tables, and only within the namespaces `catalog-config.yaml`
declares for its bucket.

`CATALOG_META_BASE_URI=memory` uses a non-persistent in-memory overlay, which is
fine for a quick local run. Persisting authored state needs an S3/MinIO overlay
(`LocalFileSystem` does not implement the conditional writes the overlay uses).

To run the frontend with hot-reload against a running API:

```sh
cd frontend && bun --bun run dev
# open http://localhost:5173 — the Vite dev proxy forwards /v1 and /ext to :8080
```

The frontend (`frontend/`) is a React + Vite + TypeScript SPA: Tailwind + shadcn/ui
primitives, TanStack Router (typed URL state), TanStack Query (server state), and
TanStack Table for the table list. API response shapes are runtime-validated with
zod schemas mirroring `catalog-core/src/types.rs`. Design rationale:
`docs/FRONTEND.md`.

```sh
cd frontend
bun --bun run typecheck   # tsc --noEmit
bun --bun run test         # vitest (format + zod schema round-trips)
bun --bun run build        # tsc --noEmit && vite build → dist/
```


### Testing against a real bucket

The committed [`catalog-config.yaml`](catalog-config.yaml) already declares the
real target bucket (`onroad-perception-datasets`, namespace
`scenario_dataset_export`, region `us-phoenix-1`), so a local run against it
needs no config change — only credentials and an endpoint override.
`onroad-perception-datasets` is an **OCI bucket accessed via its S3-compat
API**, not real AWS S3 — this needs an explicit endpoint override and OCI's
static access key/secret (the `oci.phx` AWS CLI profile), not an SSO role.

Use `export` (not per-command var prefixes) so the settings persist across the
register/sync calls below, in one shell:

```sh
# 1. kill any previously-running instance first — env is only read at startup,
#    so a running process won't pick up new exports.
pkill -f 'target/debug/catalog-api'

# 2. export everything in this shell.
eval "$(AWS_PROFILE=oci.phx aws configure export-credentials --profile oci.phx --format env)"
export AWS_ENDPOINT_URL=https://idskhu5vqvtl.compat.objectstorage.us-phoenix-1.oraclecloud.com
export AWS_VIRTUAL_HOSTED_STYLE_REQUEST=false
export AWS_DEFAULT_REGION=us-phoenix-1
export CATALOG_META_BASE_URI=memory
export CATALOG_REGISTRY_PATH=/tmp/catalog/registry.lance
export CATALOG_TTL_AUDIT_PATH=/tmp/catalog/ttl_audit.lance
export CATALOG_STORAGE_SCAN_PATH=/tmp/catalog/storage_scan.lance
export CATALOG_USERS_PATH=/tmp/catalog/users.lance

# 3. verify BEFORE starting — must print the oracle endpoint, or the server
#    will silently fall back to real AWS S3 and fail to find the bucket there.
env | grep AWS_ENDPOINT_URL

# 4. start it in this same shell.
cargo run -p catalog-api
```

Then, in another shell, register + sync exactly the table(s) you want (only
registered tables are ever synced — see [State model](#state-model-s3-is-the-sole-store)).
The id is the composite `<region>:<bucket>:<namespace>:<name>`:

```sh
curl -X PUT localhost:8080/v1/table/us-phoenix-1:onroad-perception-datasets:scenario_dataset_export:<real-table-name>
curl -X POST localhost:8080/internal/jobs/sync
curl localhost:8080/v1/table/us-phoenix-1:onroad-perception-datasets:scenario_dataset_export:<real-table-name>   # real versions come back
```

**Safety:** `CATALOG_META_BASE_URI=memory` means registering a table writes
nothing to the bucket, and the sync itself only ever does `LIST`/`GET` — never
run `POST .../ttl/apply` against this (see [RUNBOOK-TTL.md](docs/RUNBOOK-TTL.md)); it hard-deletes real data.

**Troubleshooting:**
- `Address already in use (os error 98)` — a previous run is still listening on
  8080; `pkill -f 'target/debug/catalog-api'` and restart.
- Sync error mentioning `s3.us-east-1.amazonaws.com` or `Received redirect
  without LOCATION` — the request went to real AWS instead of the OCI
  endpoint, meaning `AWS_ENDPOINT_URL` wasn't set in the process that's
  running. Re-run step 3 above to confirm, then fully restart (step 1) with all
  vars exported in the *same* shell you launch `cargo run` from — a different
  terminal tab won't have them.
- Always set `CATALOG_REGISTRY_PATH`/`CATALOG_TTL_AUDIT_PATH` explicitly (e.g.
  under `/tmp`). Left at their defaults (`_catalog/registry`, relative to the
  working directory), a run from the repo root writes a Lance dataset into the
  checkout itself.

## Configuration

All configuration is env-var driven (see `catalog-api/src/config.rs` for the
authoritative source). Defaults suit local dev / tests.

| Env var | Default | Description |
|---|---|---|
| `PORT` | — | Injected by Cloud Run; when set, the server binds `0.0.0.0:$PORT`. |
| `CATALOG_BIND_ADDR` | `0.0.0.0:8080` | Bind address when `PORT` is unset. |
| `CATALOG_CONFIG_PATH` | `catalog-config.yaml` | Path to the deployment-scoped catalog config (region, buckets, namespaces, admins) — see [Catalog config](#catalog-config-catalog-configyaml) below. |
| `CATALOG_REGISTRY_PATH` | `_catalog/registry` | URI (or path) of the derived-snapshot Lance dataset. |
| `CATALOG_TTL_AUDIT_PATH` | `_catalog/ttl_audit` | URI (or path) of the TTL audit Lance dataset. |
| `CATALOG_STORAGE_SCAN_PATH` | `_catalog/storage_scan` | URI (or path) of the storage-analysis Lance dataset (the sync's storage tail). |
| `CATALOG_USERS_PATH` | `_catalog/users` | URI (or path) of the recorded-users Lance dataset. |
| `CATALOG_META_BASE_URI` | `memory` | Base of the authored overlay objects: `memory` (in-memory, non-persistent) or `s3://bucket/prefix` (S3/MinIO). A plain filesystem path is rejected — the overlay needs conditional writes. |
| `CATALOG_CACHE_TTL_SECS` | `5` | How long the merged read view is served before it revalidates against storage. |
| `CATALOG_SYNC_CONCURRENCY` | `16` | How many versions the sync processes concurrently (in-flight LISTs + Lance dataset opens). |
| `CATALOG_SYNC_DEEP_STATS` | `all` | Which versions get the extra-IO Lance stats (`load_indices`, and the `count_rows` fallback when a manifest lacks per-fragment row counts): `all`, `latest` (each table's newest version only), or `none`. Shape/TTL-safety classification is identical in every mode; skipped versions just report `num_indices: null`. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Directory of the built SPA to serve at `/`. Absent → API-only. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager name prefix for AWS credentials (Cloud Run sets `K_SERVICE` to the service name). |

AWS S3 credentials: on Cloud Run they are read from Secret Manager at startup
(there is no ambient AWS credential — see [`ARCHITECTURE.md`](ARCHITECTURE.md)
and the [deploy runbook](docs/RUNBOOK-DEPLOY.md)). Locally, the standard `AWS_*`
env vars (`AWS_ENDPOINT_URL`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
`AWS_ALLOW_HTTP`, `AWS_VIRTUAL_HOSTED_STYLE_REQUEST`, `AWS_DEFAULT_REGION`) are
consumed by `object_store::parse_url_opts`, so an `AWS_ENDPOINT_URL` pointed at
MinIO exercises the real conditional-write path.

### Catalog config (`catalog-config.yaml`)

The service is regional: it runs colocated with the target buckets it
serves, and its scope — region, buckets, and each bucket's registered
namespace prefixes — is declared in a committed YAML file, baked into the
image (path overridable via `CATALOG_CONFIG_PATH`). An operator changes scope
(adds a bucket, adds a namespace) with a PR + redeploy, not a runtime config
flip. `admins` is a plain email allowlist overlaid onto recorded users at read time
(never persisted per-user) — see [`GET /ext/v1/users`](#rest-api-surface) and
[`GET /ext/v1/me`](#rest-api-surface). The repo-root
[`catalog-config.yaml`](catalog-config.yaml) currently declares:

```yaml
region: us-phoenix-1

buckets:
  - name: onroad-perception-datasets
    namespaces:
      - scenario_dataset_export

admins:
  - shiyan.xu@applied.co
```

Table ids are region-first composites, `<region>:<bucket>:<namespace>:<name>`
(colon-delimited — see `catalog-core/src/table_id.rs`), so ids stay unambiguous
if the catalog ever aggregates multiple regional deployments. `DeclareTable`
rejects an id whose `(bucket, namespace)` isn't declared in this file.

## REST API surface

The public router (`/v1` + `/ext/v1`) is wired in `catalog-api/src/api.rs`.
Reads serve the merged view (derived snapshot + authored overlay), lazily
revalidated. Any instance can serve any request — reads and mutations alike —
because the snapshot is written last-wins and the overlay is serialized per
table by S3's conditional writes.

- `GET /healthz` — liveness (`ok` from boot).
- `GET /readyz` — readiness; `503` until the first successful view load, then `200`.
- `GET /v1/namespaces` — list distinct namespaces (derived from cataloged tables).
- `GET /v1/namespaces/:id` — describe a namespace (id is `:`-joined `bucket:prefix[:prefix...]`).
- `GET /v1/tables` — list table ids.
- `GET /v1/table/:id` — describe a table (full `TableEntry`).
- `PUT /v1/table/:id` — register a table (set `owner`/`ttl_policy`; idempotent). Registering adds it to the synced set; its versions appear on the next sync. `:id`'s `(bucket, namespace)` must be declared in `catalog-config.yaml`.
- `DELETE /v1/table/:id` — deregister (deletes the authored overlay). The table leaves the synced set and the next sync drops its derived entry. S3 data is untouched.
- `GET /ext/v1/tables?expand=...` — enriched listing (full detail; no pagination).
- `GET /ext/v1/storage` — the current bucket-storage breakdown from the sync's storage-analysis tail (registered namespaces sized from the registry; other top-level prefixes reported unexplored).
- `GET /ext/v1/me` — the caller's identity as seen by IAP, with its role resolved against `catalog-config.yaml`'s `admins` list.
- `GET /ext/v1/users` — users recorded by the identity layer, with `role` overlaid from `admins` at read time.
- `GET /ext/v1/tables/:id/versions/:vid` — single version detail.
- `GET /ext/v1/tables/:id/versions/:vid/aux/sample?name=&limit=` — read up to 100 sample rows from an auxiliary table (lance via the lance scanner, parquet dirs via DataFusion; read-only).
- `PUT /ext/v1/tables/:id/versions/:vid/protect` — set/clear a version's TTL-exempt `protected` flag.
- `GET /ext/v1/tables/:id/ttl/dryrun` — list TTL-eligible versions + reclaimable bytes (read-only).
- `POST /ext/v1/tables/:id/ttl/apply` — hard-delete eligible versions (irreversible, audited).
- `GET /ext/v1/tables/:id/ttl/audit` — read a table's TTL audit log.
- `POST /internal/jobs/sync` — run one sync pass. Triggered by Cloud Scheduler; idempotent under retry/double-fire.

## Observability

Structured JSON logs (sync summaries, per-table sync failures, TTL outcomes)
go to stdout for Cloud Logging. HTTP request/latency (RED) metrics come from
Cloud Run's built-in Cloud Monitoring; there is no Prometheus endpoint.
`RUST_LOG` tunes log verbosity (default `info`, with Lance's per-operation
chatter quieted to `warn`).

## Deployment

Deployed to Cloud Run via Apps Platform (`apps-platform app deploy --local`),
with the sync driven by Cloud Scheduler and AWS credentials in Secret Manager.
See [`docs/RUNBOOK-DEPLOY.md`](docs/RUNBOOK-DEPLOY.md).
