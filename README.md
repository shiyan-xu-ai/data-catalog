# Data Catalog

A [Lance REST catalog](https://lance.org/format/catalog/rest/) backed by
Postgres. Operators register existing Lance datasets by URI under nested
namespaces, then browse, query, and inspect them through the bundled web UI or
any Lance namespace client.

Postgres records namespaces, table registrations, and each table's current
version. Each dataset's Lance manifest stays authoritative for the schema,
statistics, and data of every version. The catalog never writes to a registered
dataset.

## Architecture

```
   Browser (React UI)            Lance namespace clients
           |                               |
           +---------------+---------------+
                           | HTTP, one origin (platform IAP in front)
                           v
 +--------------------- catalog-api (axum binary) ---------------------+
 |  /v1/*          Lance namespace spec routes                          |
 |  /catalog/v1/*  catalog routes: storage, timeline, SQL, refresh,     |
 |                 media presign                                        |
 |  /              static UI from CATALOG_WEBUI_DIR                     |
 +-----------------------------------+----------------------------------+
                                     | LanceNamespace trait
 +-----------------------------------v----------------------------------+
 |  catalog-store: PostgresNamespace                                    |
 |  per-version cache of opened datasets and stats (Lance session)      |
 +------------------+--------------------------------+------------------+
                    | sqlx                           | lance, read-only
                    v                                v
        +-----------------------+      +--------------------------------+
        | Postgres              |      | Object storage (S3 / OCI)      |
        |  namespaces           |      |  Lance datasets: manifests,    |
        |  tables (location,    |      |  data files, indices, blobs    |
        |  properties, current  |      +--------------------------------+
        |  version)             |                     ^
        +-----------------------+                     | presigned URL,
                                                      | ranged reads
                                       Browser media playback
```

- A read that names no version answers for the table's **current version**:
  the newest version at registration, advanced only by
  `POST /catalog/v1/table/:id/refresh` or re-registration.
- One container on Cloud Run serves the API and the UI. Media bytes go from
  object storage straight to the browser; the server only signs URLs.

[ARCHITECTURE.md](ARCHITECTURE.md) covers the data model, caching, SQL, and
deployment in detail.

## API

Path ids are name segments joined by `$`; the root namespace is `$`. Table
`prod.analytics.users` is `prod$analytics$users`. Bodies use the
[Lance namespace spec](https://lance.org/format/namespace/) models. Errors are
`{"error": "...", "code": N}` with the spec error code. Paginated routes take
`limit` and `page_token` (the last name or version number of the previous
page).

### Lance namespace spec (`/v1`)

| Method | Path | Returns |
|---|---|---|
| GET | `/v1/namespace/:id/list` | Child namespaces. |
| POST | `/v1/namespace/:id/create` | Creates a namespace; its parent must exist. |
| POST | `/v1/namespace/:id/describe` | Namespace properties. |
| POST | `/v1/namespace/:id/exists` | 200, or 404. |
| POST | `/v1/namespace/:id/drop` | Drops an empty namespace; 409 if it has children. |
| GET | `/v1/namespace/:id/table/list` | Tables in a namespace. |
| GET | `/v1/table` | Every table id. |
| POST | `/v1/table/:id/register` | Registers the Lance dataset at `location`; it must open as Lance, and its newest version becomes current. `"mode": "overwrite"` replaces the location and properties, keeps `catalog.registered_at`, and advances `catalog.updated_at`. |
| POST | `/v1/table/:id/describe` | Location and properties from Postgres. With `?load_detailed_metadata=true`, a body `version`, or a body `tag` (resolved to its version; not combinable with `version`), also opens that version and returns `version`, `schema`, `stats`, and `metadata` (manifest summary, `table_metadata.*`, `config.*`, `lance.data_storage_version`). |
| POST | `/v1/table/:id/exists` | 200, or 404. With a body `version`, 404 also when that version does not exist. |
| POST | `/v1/table/:id/deregister` | Removes the registration; the data stays. |
| POST | `/v1/table/:id/stats` | Total bytes, rows, index count, and rows-per-fragment statistics. |
| POST | `/v1/table/:id/count_rows` | Live rows as a bare integer, at the current version or a body `version`; an optional `predicate` (SQL filter) scans the filtered columns. |
| POST | `/v1/table/:id/version/list` | Versions with their manifest summaries, including versions newer than the current one (`?descending=true` for newest first). |
| POST | `/v1/table/:id/version/describe` | One version. |
| POST | `/v1/table/:id/index/list` | Indices with type and columns. |
| POST | `/v1/table/:id/index/:name/stats` | One index's size and indexed versus unindexed rows. |
| POST | `/v1/table/:id/tags/list` | Tags. |
| POST | `/v1/table/:id/tags/version` | The version a body `tag` points at; 404 for an unknown tag. |
| POST | `/v1/table/:id/branches/list` | Branches. |

Every other spec route returns `Unsupported` (code 0, HTTP 406): data writes,
commits, tag, branch, and index writes, transactions, rename, restore,
cascading drop, and credential vending.

### Catalog routes (`/catalog/v1`)

| Method | Path | Returns |
|---|---|---|
| GET | `/catalog/v1/table/:id/column-storage` | Bytes on disk per top-level column, excluding blob payloads. |
| GET | `/catalog/v1/table/:id/storage` | Bytes and objects by category (data, blob, index, deletion, manifest, transaction, other) for the current version and for everything under the location, plus an estimated monthly cost. |
| GET | `/catalog/v1/table/:id/fragment-measures` | Per-fragment data files, data bytes, and deleted rows (min, p25, median, mean, p75, p99, max), from the manifest. |
| GET | `/catalog/v1/table/:id/timeline` | One event per retained version, newest first (`page_token`, `limit` up to 200): operation, details, manifest summary, writer, transaction properties, tags. |
| GET | `/catalog/v1/table/:id/version/:version/files` | The manifest and transaction files of one version, decoded from protobuf to JSON. |
| POST | `/catalog/v1/table/:id/refresh` | Moves the current version to the newest committed one; returns `previous_version` and `current_version`. |
| POST | `/catalog/v1/table/:id/sql` | Runs one read-only SQL query (`{"sql": "...", "version": N}`). The table's name reads `version` (default: current); `"<name>@v<N>"` reads version N. Returns columns (`name`, Arrow `type`, field `metadata`) and rows, capped at `CATALOG_SQL_MAX_ROWS` and flagged `truncated`. |
| GET | `/catalog/v1/media/presign?uri=s3://...` | Presigned GET URLs (`url`, `download_url`, `expires_in_secs`) for any object the catalog's credentials can read. |

### Health

| Method | Path | Returns |
|---|---|---|
| GET | `/healthz` | 200 while the process runs. |
| GET | `/readyz` | 200 once Postgres answers, else 503. |

The service has no authentication of its own; the platform's IAP guards
deployed access.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string. |
| `CATALOG_DB_SCHEMA` | unset | Schema for catalog tables and migrations; created if missing. |
| `PORT` / `CATALOG_BIND_ADDR` | unset / `0.0.0.0:8080` | Listen address; `PORT` (Cloud Run) wins. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Built UI; without an `index.html`, only the API is served. |
| `CATALOG_VERTICALS` | unset | Comma-separated top-level namespaces allowed (deployed: `automotive,trucking,mining,robotics`). Unset allows any. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager prefix for AWS credentials on Cloud Run. |
| `CATALOG_STORAGE_PRICE_PER_GB_MONTH` | `0.0133875` | USD per GB-month for the cost estimate. |
| `CATALOG_STORAGE_PRICE_TIER` | `OCI Object Storage Active (contracted)` | Label shown with the estimate. |
| `CATALOG_SQL_MAX_ROWS` | `1000` | Rows a query returns before truncation. |
| `CATALOG_SQL_TIMEOUT_SECS` | `60` | Per-query time limit. |
| `CATALOG_SQL_MEMORY_LIMIT_MB` | `512` | Per-query memory pool. |
| `CATALOG_SQL_MAX_CONCURRENT` | `2` | Concurrent queries per instance; more get 429. |
| `CATALOG_CACHE_MEMORY_LIMIT_MB` | `256` | Lance session cache per instance. |
| `CATALOG_MEDIA_URL_TTL_MINUTES` | `60` | Lifetime of presigned media URLs. |
| `RUST_LOG` | `info,lance=warn` | Log filter; logs are JSON. |

Datasets open with the standard `AWS_*` variables, including
`AWS_ENDPOINT_URL` for S3-compatible stores such as OCI. On Cloud Run the
credentials come from Secret Manager unless `AWS_ACCESS_KEY_ID` is set.

## Development

Needs Rust (pinned in `rust-toolchain.toml`), `protoc`, Docker, and Bun.

```sh
docker run -d --name catalog-pg -e POSTGRES_PASSWORD=pg -p 55432:5432 postgres:17
export DATABASE_URL=postgres://postgres:pg@127.0.0.1:55432/postgres
(cd frontend && bun install && bun run build)       # UI served from frontend/dist
cargo run --bin catalog-api                          # http://localhost:8080
CATALOG_TEST_DATABASE_URL=$DATABASE_URL cargo test --workspace --all-targets
```

For UI work, `cd frontend && bun run dev` serves on port 5173 and proxies
`/v1` and `/catalog` to port 8080.

## Documentation

- [AGENTS.md](AGENTS.md): working rules for contributors and coding agents.
- [ARCHITECTURE.md](ARCHITECTURE.md): data model, current version, caching, SQL, deployment.
- [docs/FRONTEND.md](docs/FRONTEND.md): UI behavior and frontend code.
- [docs/RUNBOOK-DEPLOY.md](docs/RUNBOOK-DEPLOY.md): deploying and operating on Apps Platform.
- `docs/RELEASE-v*.md`: release notes; `docs/plans/`: past implementation plans.
