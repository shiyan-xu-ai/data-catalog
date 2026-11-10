# Data Catalog

A regional metadata catalog for Lance tables in S3-compatible object storage. Operators register the tables to track, then browse their versions, schemas, storage sizes, row counts, and auxiliary datasets through a REST API and React UI. Unregistered tables are ignored.

## Architecture

The Rust API and static UI run together in a Cloud Run container. Catalog state lives in object storage:

- **Registry:** a Lance dataset containing derived table and version metadata.
- **Registration metadata:** one JSON object per table containing its optional owner. Conditional writes protect concurrent updates.

Cloud Scheduler or an operator calls the sync endpoint. The catalog **pulls** metadata from registered tables and writes the registry snapshot; writers do not push stats. New versions appear after a successful sync. Clean versions are carried forward assuming timestamp directories are immutable, so modifying one in place can leave its stats stale. The API merges registry and registration metadata into a read view, cached for five seconds by default.

The workspace contains `catalog-core` (types and registry), `catalog-store` (object-store access and sync), `catalog-api` (HTTP server and sync orchestration), and `frontend` (React UI). See [architecture](ARCHITECTURE.md) for details and the [deployment runbook](docs/RUNBOOK-DEPLOY.md) for setup.

## API

Table IDs use `<region>:<bucket>:<namespace>:<name>`; nested namespaces use additional colon-separated segments. Registration requires a region, bucket, and namespace declared in `catalog-config.yaml`. An optional registration body sets the owner, for example `{"owner":"team@example.com"}`.

| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/healthz` | Liveness. |
| GET | `/readyz` | Readiness; returns 503 until the first successful view load. |
| GET | `/v1/namespaces` | List namespaces represented by cataloged tables. |
| GET | `/v1/namespaces/:id` | Describe a `bucket:namespace` ID. |
| GET | `/v1/tables` | List registered table IDs. |
| GET | `/v1/table/:id` | Describe a table. |
| PUT | `/v1/table/:id` | Register a table or set its owner. Versions appear on the next sync. |
| DELETE | `/v1/table/:id` | Deregister a table without deleting source data. |
| GET | `/ext/v1/tables` | List full table entries; no server-side pagination. |
| GET | `/ext/v1/tables/:id/versions/:vid` | Describe a version. |
| GET | `/ext/v1/tables/:id/versions/:vid/aux/sample?name=&limit=` | Sample Lance or Parquet auxiliary rows; default 20, maximum 100. |
| POST | `/internal/jobs/sync` | Run a sync pass. |

The API router is defined in [`catalog-api/src/api.rs`](catalog-api/src/api.rs). Platform IAP protects deployed access; the application does not enforce authentication itself.

## Configuration

[`catalog-config.yaml`](catalog-config.yaml) declares the deployment's region, buckets, and namespaces. Scope changes require a config change and redeploy. The committed config targets `scenario_dataset_export` in the `onroad-perception-datasets` bucket, region `us-phoenix-1`.

| Environment variable | Default | Purpose |
|---|---|---|
| `PORT` | unset | Cloud Run port; overrides `CATALOG_BIND_ADDR`. |
| `CATALOG_BIND_ADDR` | `0.0.0.0:8080` | Server bind address when `PORT` is unset. |
| `CATALOG_CONFIG_PATH` | `catalog-config.yaml` | Region, buckets, and namespace configuration. |
| `CATALOG_REGISTRY_PATH` | `_catalog/registry` | Lance registry path or URI. |
| `CATALOG_META_BASE_URI` | `memory` | Registration metadata store: `memory` or an S3 URI. Memory is non-persistent; a filesystem path is not supported. |
| `CATALOG_CACHE_TTL_SECS` | `5` | Merged read-view cache lifetime in seconds. |
| `CATALOG_SYNC_CONCURRENCY` | `16` | Maximum concurrent version syncs. |
| `CATALOG_SYNC_DEEP_STATS` | `all` | Extra Lance stats for `all`, `latest`, or `none` of newly processed versions. Skipped index counts and fallback row counts may be null. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Built UI directory; absent means API-only. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager prefix for AWS credentials on Cloud Run. |

Local S3-compatible access uses standard `AWS_*` environment variables, including `AWS_ENDPOINT_URL` and credentials. On Cloud Run, credentials are read from Secret Manager unless already present in the environment. See [deployment](docs/RUNBOOK-DEPLOY.md) and [frontend development](docs/FRONTEND.md).
