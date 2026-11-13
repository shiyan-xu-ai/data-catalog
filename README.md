# Data Catalog

A [Lance REST catalog](https://lance.org/format/catalog/rest/) backed by Postgres. Operators create nested namespaces and register existing Lance datasets by URI. The catalog stores each table's name, location, properties, and current version; the dataset's Lance manifest stays authoritative for each version's schema, stats, and contents. The bundled React UI and standard Lance namespace clients use the same routes.

- [Architecture](ARCHITECTURE.md): design, data model, current version and caching.
- [Frontend](docs/FRONTEND.md): UI behavior and development.
- [Deployment runbook](docs/RUNBOOK-DEPLOY.md).

## API

Identifiers in paths are segments joined by `$`; the root namespace is `$`. For example, table `prod.analytics.users` is `prod$analytics$users`. Names must be non-empty and contain no `$` or control characters. Request and response bodies use the spec's models. Errors are `{"error": "...", "code": N}` with the spec error code. Paginated routes take `limit` and an exclusive `page_token`: the last name, or version number, of the previous page. Namespace and table lists never open datasets.

| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/v1/namespace/:id/list` | List child namespaces. |
| POST | `/v1/namespace/:id/create` | Create a namespace; its parent must exist. |
| POST | `/v1/namespace/:id/describe` | Return namespace properties. |
| POST | `/v1/namespace/:id/exists` | 200 if the namespace exists, else 404. |
| POST | `/v1/namespace/:id/drop` | Drop an empty namespace; 409 if not empty. |
| GET | `/v1/namespace/:id/table/list` | List tables in a namespace. |
| GET | `/v1/table` | List all table ids. |
| POST | `/v1/table/:id/register` | Register the existing Lance dataset at `location`. The URI must open as Lance, so an unreadable URI never becomes a table; its newest version becomes current. Optional `owner` and `description` properties; `catalog.*` keys are reserved. Re-registering with `"mode": "overwrite"` keeps `catalog.registered_at` and advances `catalog.updated_at`. |
| POST | `/v1/table/:id/describe` | Location and properties from Postgres alone, including server-managed `catalog.registered_at` and `catalog.updated_at`. With `?load_detailed_metadata=true` or a body `version`, also opens that version (default: current) and returns `version`, `schema`, `stats`, and `metadata`: the manifest summary, `table_metadata.*` (Lance `Dataset::metadata`), `config.*` (Lance `Dataset::config`), and `lance.data_storage_version`. |
| POST | `/v1/table/:id/exists` | 200 if the table exists, else 404. |
| POST | `/v1/table/:id/deregister` | Remove the registration; source data is not deleted. |
| POST | `/v1/table/:id/stats` | Total bytes, rows, index count, and fragment length stats. |
| POST | `/v1/table/:id/version/list`, `/version/describe` | Version history with each manifest's summary (`?descending=true` for newest first), including versions newer than the current one. |
| POST | `/v1/table/:id/index/list`, `/index/:name/stats` | Indices with type, columns, size, and coverage. |
| POST | `/v1/table/:id/tags/list`, `/branches/list` | Tags and branches. |
| GET | `/catalog/v1/table/:id/column-storage` | Bytes on disk per top-level column, excluding blob payload files. |
| GET | `/catalog/v1/table/:id/storage` | Bytes and objects under the table location by category (data, blob, index, deletion, manifest, transaction, other), both for the objects the current version references (`current`: its data files and their blob sidecars, deletion files, index directories, manifest, and transaction file) and for every object (`on_storage`); the current version's data and index bytes; and an estimated monthly cost of every object. |
| GET | `/catalog/v1/table/:id/fragment-measures` | Per-fragment data files, data bytes, and deleted rows (min, p25, median, mean, p75, p99, max each), read from the manifest alone; a measure is null when some fragment does not record it. |
| GET | `/catalog/v1/media/presign?uri=` | Presigned https GET URLs (`{url, download_url, expires_in_secs}`) for any `s3://bucket/key` object the catalog's storage credentials can read: `url` reads it (byte ranges allowed), `download_url` serves it as an attachment. The UI plays and downloads videos with them. Signing makes no storage request, so a missing or unreadable object fails only when a URL is fetched; other URIs get 400. |
| POST | `/catalog/v1/table/:id/refresh` | Advance the current version to the newest version at the table location; returns `previous_version` and `current_version`. |
| POST | `/catalog/v1/table/:id/sql` | Run a read-only SQL query (`{"sql": "...", "version": N}`) against the table, addressed by its name. `version` (default: current) picks which version the plain name reads; `"<name>@v<N>"` reads version N in the same query, for comparisons. DDL, DML, and `COPY` are rejected; results are capped at `CATALOG_SQL_MAX_ROWS` rows and flagged `truncated`. Each result column has its `name`, Arrow `type`, and, when present, Arrow field `metadata` (blob columns read as descriptors carry `lance-encoding:blob`). |
| GET | `/catalog/v1/table/:id/timeline` | One event per retained version, newest first (`page_token`, `limit` up to 200), read from each version's transaction and manifest: the Lance operation (append, delete, update, overwrite, compaction, index build, schema or config change, restore), its recorded details, the manifest summary, the writer library version, transaction properties, and tags. |
| GET | `/catalog/v1/table/:id/version/:version/files` | The files that record one version, decoded from protobuf to JSON: the manifest file (`_versions/…`) with its manifest, index section, and inline transaction, plus the separate `_transactions/…` file older writers wrote. Each file lists its sections with byte offsets; byte fields over 64 bytes show only their length. Reads only those files. |
| GET | `/healthz`, `/readyz` | Liveness, and readiness (503 until Postgres answers). |

Reads that name no version answer for the table's current version: the newest version at registration, advanced only by the refresh route or re-registration (see [current version and caching](ARCHITECTURE.md#current-version-and-caching)). `/catalog/v1` routes carry data no spec model holds.

The catalog never writes to registered datasets. Every other spec route returns `Unsupported` (code 0, HTTP 406). Not implemented: data operations, catalog-managed versions and commits, tag, branch, and index writes, transactions, rename and restore, reading tags or non-main branches, cascading namespace drop, credential vending, and authentication. Platform IAP protects deployed access.

## Configuration

| Environment variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string. On Cloud Run, `docker-entrypoint.sh` derives it. |
| `CATALOG_DB_SCHEMA` | unset | Schema for catalog tables and migrations, created if missing. On Cloud Run, `docker-entrypoint.sh` defaults it to the service name with `-` replaced by `_`. |
| `PORT` | unset | Cloud Run port; overrides `CATALOG_BIND_ADDR`. |
| `CATALOG_BIND_ADDR` | `0.0.0.0:8080` | Bind address when `PORT` is unset. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Built UI directory (`/app/webui` in the container); without an `index.html` there, only the API is served. |
| `CATALOG_VERTICALS` | unset | Comma-separated business verticals allowed as top-level namespaces (deployed: `automotive,trucking,mining,robotics`), so the tree reads `<vertical>/<domain>/<table>`. Creating any other top-level namespace returns 400; deeper levels are unrestricted, and existing top-level namespaces stay readable and droppable. Unset allows any. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager prefix for AWS credentials on Cloud Run. |
| `CATALOG_STORAGE_PRICE_PER_GB_MONTH` | `0.0133875` | USD per decimal GB-month for cost estimates (OCI Active tier, contracted rate). |
| `CATALOG_STORAGE_PRICE_TIER` | `OCI Object Storage Active (contracted)` | Label shown next to the cost estimate. |
| `CATALOG_SQL_MAX_ROWS` | `1000` | Maximum rows a SQL query returns; larger results are truncated and flagged. |
| `CATALOG_SQL_TIMEOUT_SECS` | `60` | Per-query time limit. |
| `CATALOG_SQL_MEMORY_LIMIT_MB` | `512` | Per-query DataFusion memory pool. |
| `CATALOG_SQL_MAX_CONCURRENT` | `2` | Concurrent SQL queries per instance; extra requests get 429 (Throttling). |
| `CATALOG_CACHE_MEMORY_LIMIT_MB` | `256` | Lance session cache budget per instance (see [caching](ARCHITECTURE.md#current-version-and-caching)). |
| `CATALOG_MEDIA_URL_TTL_MINUTES` | `60` | Lifetime of presigned media URLs. |
| `RUST_LOG` | `info,lance=warn` | Log filter; logs are JSON. |

Datasets on S3 or S3-compatible stores open with the standard `AWS_*` variables, including `AWS_ENDPOINT_URL`. On Cloud Run, credentials come from Secret Manager unless `AWS_ACCESS_KEY_ID` is already set. Credentials are never returned to clients.

## Development

```sh
docker run -d --name catalog-pg -e POSTGRES_PASSWORD=pg -p 55432:5432 postgres:17
export DATABASE_URL=postgres://postgres:pg@127.0.0.1:55432/postgres
cargo run --bin catalog-api
CATALOG_TEST_DATABASE_URL=$DATABASE_URL cargo test --workspace
```

See [frontend development](docs/FRONTEND.md#development) for the UI dev server.
