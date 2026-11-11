# Data Catalog

A [Lance REST catalog](https://lance.org/format/catalog/rest/) backed by Postgres. Operators create nested namespaces and register existing Lance datasets by URI. The catalog stores each table's name, location, and properties, while the dataset's Lance manifest remains authoritative for version, schema, and stats. A React UI and standard Lance namespace clients use the same spec routes.

See [architecture](ARCHITECTURE.md) and the [deployment runbook](docs/RUNBOOK-DEPLOY.md).

## API

Identifiers in paths are segments joined by `$`; the root namespace is `$`. For example, table `prod.analytics.users` is `prod$analytics$users`. Request and response bodies use the spec's models, and errors are returned as `{"error": "...", "code": N}`.

| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/v1/namespace/:id/list` | List child namespaces (`page_token`, `limit`). |
| POST | `/v1/namespace/:id/create` | Create a namespace; its parent must exist. |
| POST | `/v1/namespace/:id/describe` | Return namespace properties. |
| POST | `/v1/namespace/:id/exists` | Return 200 if the namespace exists, or 404. |
| POST | `/v1/namespace/:id/drop` | Drop an empty namespace; returns 409 if not empty. |
| GET | `/v1/namespace/:id/table/list` | List tables in a namespace. |
| GET | `/v1/table` | List all tables. |
| POST | `/v1/table/:id/register` | Register an existing Lance dataset at `location`. The URI must open as Lance. Optional `owner` and `description` properties; `catalog.*` keys are reserved. |
| POST | `/v1/table/:id/describe` | Return location and properties, including server-managed `catalog.registered_at` and `catalog.updated_at`. With `?load_detailed_metadata=true`, also return version, schema, stats, and `metadata` (manifest summary, `table_metadata.*` from Lance `Dataset::metadata`, `config.*` from Lance `Dataset::config`, `lance.data_storage_version`). |
| POST | `/v1/table/:id/exists` | Return 200 if the table exists, or 404. |
| POST | `/v1/table/:id/deregister` | Remove the registration. Source data is not deleted. |
| POST | `/v1/table/:id/stats` | Total bytes, rows, index count, and fragment length stats. Reads field statistics, so it is slow on large tables. |
| POST | `/v1/table/:id/version/list`, `/version/describe` | Version history with each manifest's summary. |
| POST | `/v1/table/:id/index/list`, `/index/:name/stats` | Indices with type, columns, size, and coverage. |
| POST | `/v1/table/:id/tags/list`, `/branches/list` | Tags and branches. |
| GET | `/catalog/v1/table/:id/column-storage` | Bytes on disk per top-level column of the current version (catalog route; no spec model carries it). Excludes blob payload files. Slow on large tables. |
| GET | `/catalog/v1/table/:id/storage` | Current-version data and index bytes, total bytes on object storage under the table location by category (data, blob, index, deletion, manifest, transaction, other), and an estimated monthly storage cost. Lists the dataset prefix, so it is slow on large tables. |
| POST | `/catalog/v1/table/:id/sql` | Run a read-only SQL query (`{"sql": "...", "version": N}`) against the table, addressed by its table name. `version` (default latest) picks which version the plain name reads; `"<name>@v<N>"` reads version N inside the same query, for comparisons. DDL, DML, and `COPY` are rejected; results are capped at `CATALOG_SQL_MAX_ROWS` rows and flagged `truncated`. |
| GET | `/catalog/v1/table/:id/timeline` | One event per retained version, newest first (`page_token`, `limit` up to 200): the Lance operation (append, delete, update, overwrite, compaction, index build, schema or config change, restore), its recorded details, the manifest summary, the writer library version, transaction properties, and tags. Reads each version's transaction and manifest. |
| GET | `/healthz`, `/readyz` | Liveness, and readiness (requires Postgres). |

The catalog never writes to registered datasets. Other spec operations, including data operations, version/tag/branch/index writes, and transactions, return `Unsupported` (code 0, HTTP 406). Platform IAP protects deployed access, and the application performs no authentication itself.

## Configuration

| Environment variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string. On Cloud Run, `docker-entrypoint.sh` derives it. |
| `CATALOG_DB_SCHEMA` | unset | Schema for catalog tables and migrations, created if missing. |
| `PORT` | unset | Cloud Run port; overrides `CATALOG_BIND_ADDR`. |
| `CATALOG_BIND_ADDR` | `0.0.0.0:8080` | Bind address when `PORT` is unset. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Built UI directory; if absent, only the API is served. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager prefix for AWS credentials on Cloud Run. |
| `CATALOG_STORAGE_PRICE_PER_GB_MONTH` | `0.0133875` | Storage price per decimal GB-month used for cost estimates (OCI Active tier, contracted rate). |
| `CATALOG_STORAGE_PRICE_TIER` | `OCI Object Storage Active (contracted)` | Label shown next to the cost estimate. |
| `CATALOG_SQL_MAX_ROWS` | `1000` | Maximum rows a SQL query returns; larger results are truncated and flagged. |
| `CATALOG_SQL_TIMEOUT_SECS` | `60` | Per-query time limit. |
| `CATALOG_SQL_MEMORY_LIMIT_MB` | `512` | Per-query DataFusion memory pool. |
| `CATALOG_SQL_MAX_CONCURRENT` | `2` | Concurrent SQL queries per instance; extra requests get 429 (Throttling). |

Datasets on S3 or S3-compatible stores are opened with the standard `AWS_*` variables, including `AWS_ENDPOINT_URL`. On Cloud Run, credentials come from Secret Manager unless they are already in the environment.

## Development

```sh
docker run -d --name catalog-pg -e POSTGRES_PASSWORD=pg -p 55432:5432 postgres:17
export DATABASE_URL=postgres://postgres:pg@127.0.0.1:55432/postgres
cargo run --bin catalog-api
CATALOG_TEST_DATABASE_URL=$DATABASE_URL cargo test --workspace
```

See [frontend development](docs/FRONTEND.md) for the UI dev server.
