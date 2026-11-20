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
`prod.analytics.users` is `prod$analytics$users`. A `?delimiter=` other than
`$` is rejected as invalid input. Bodies use the
[Lance namespace spec](https://lance.org/format/namespace/) models. Errors are
`{"error": "...", "code": N}` with the spec error code. Paginated routes take
`limit` and `page_token` (the last name or version number of the previous
page).

### Lance namespace spec (`/v1`)

Each route implements the linked spec operation. Links go to the operation's
request model, or to the REST catalog spec or OpenAPI file where the spec site
has no model page.

| Method | Path | Spec operation | Returns |
|---|---|---|---|
| GET | `/v1/namespace/:id/list` | [ListNamespaces][ListNamespaces] | Child namespaces. |
| POST | `/v1/namespace/:id/create` | [CreateNamespace][CreateNamespace] | Creates a namespace; its parent must exist. |
| POST | `/v1/namespace/:id/describe` | [DescribeNamespace][DescribeNamespace] | Namespace properties. |
| POST | `/v1/namespace/:id/exists` | [NamespaceExists][NamespaceExists] | 200, or 404. |
| POST | `/v1/namespace/:id/drop` | [DropNamespace][DropNamespace] | Drops an empty namespace; 409 if it has children. |
| GET | `/v1/namespace/:id/table/list` | [ListTables][ListTables] | Tables in a namespace. |
| GET | `/v1/table` | [ListAllTables][ListAllTables] | Every table id. |
| POST | `/v1/table/:id/register` | [RegisterTable][RegisterTable] | Registers the Lance dataset at `location`; it must open as Lance and fall under `CATALOG_TABLE_LOCATIONS` (else PermissionDenied, 403), and its newest version becomes current. `"mode": "overwrite"` replaces the location and properties, keeps `catalog.registered_at`, and advances `catalog.updated_at`. |
| POST | `/v1/table/:id/describe` | [DescribeTable][DescribeTable] | Location and properties from Postgres. With `?load_detailed_metadata=true`, a body `version`, or a body `tag` (resolved to its version; not combinable with `version`), also opens that version and returns `version`, `schema`, `stats`, and `metadata` (manifest summary, `table_metadata.*`, `config.*`, `lance.data_storage_version`). |
| POST | `/v1/table/:id/exists` | [TableExists][TableExists] | 200, or 404. With a body `version`, 404 also when that version does not exist. |
| POST | `/v1/table/:id/deregister` | [DeregisterTable][DeregisterTable] | Removes the registration; the data stays. |
| POST | `/v1/table/:id/stats` | [GetTableStats][GetTableStats] | Total bytes, rows, index count, and rows-per-fragment statistics. |
| POST | `/v1/table/:id/count_rows` | [CountTableRows][CountTableRows] | Live rows as a bare integer, at the current version or a body `version`; an optional `predicate` (SQL filter) scans the filtered columns. |
| POST | `/v1/table/:id/version/list` | [ListTableVersions][ListTableVersions] | Versions with their manifest summaries, including versions newer than the current one (`?descending=true` for newest first). |
| POST | `/v1/table/:id/version/describe` | [DescribeTableVersion][DescribeTableVersion] | One version. |
| POST | `/v1/table/:id/index/list` | [ListTableIndices][ListTableIndices] | Indices with type and columns. |
| POST | `/v1/table/:id/index/:name/stats` | [DescribeTableIndexStats][DescribeTableIndexStats] | One index's size and indexed versus unindexed rows. |
| POST | `/v1/table/:id/tags/list` | [ListTableTags][ListTableTags] | Tags. |
| POST | `/v1/table/:id/tags/version` | [GetTableTagVersion][GetTableTagVersion] | The version a body `tag` points at; 404 for an unknown tag. |
| POST | `/v1/table/:id/branches/list` | [ListTableBranches][ListTableBranches] | Branches. |

[ListNamespaces]: https://lance.org/format/namespace/operations/models/ListNamespacesRequest/
[CreateNamespace]: https://lance.org/format/namespace/operations/models/CreateNamespaceRequest/
[DescribeNamespace]: https://lance.org/format/namespace/operations/models/DescribeNamespaceRequest/
[NamespaceExists]: https://lance.org/format/namespace/operations/models/NamespaceExistsRequest/
[DropNamespace]: https://lance.org/format/namespace/operations/models/DropNamespaceRequest/
[ListTables]: https://lance.org/format/namespace/operations/models/ListTablesRequest/
[ListAllTables]: https://lance.org/format/catalog/rest/#listalltables
[RegisterTable]: https://lance.org/format/namespace/operations/models/RegisterTableRequest/
[DescribeTable]: https://lance.org/format/namespace/operations/models/DescribeTableRequest/
[TableExists]: https://lance.org/format/namespace/operations/models/TableExistsRequest/
[DeregisterTable]: https://lance.org/format/namespace/operations/models/DeregisterTableRequest/
[GetTableStats]: https://lance.org/format/namespace/operations/models/GetTableStatsRequest/
[CountTableRows]: https://lance.org/format/namespace/operations/models/CountTableRowsRequest/
[ListTableVersions]: https://lance.org/format/namespace/operations/models/ListTableVersionsRequest/
[DescribeTableVersion]: https://lance.org/format/namespace/operations/models/DescribeTableVersionRequest/
[ListTableIndices]: https://lance.org/format/namespace/operations/models/ListTableIndicesRequest/
[DescribeTableIndexStats]: https://lance.org/format/namespace/operations/models/DescribeTableIndexStatsRequest/
[ListTableTags]: https://lance.org/format/namespace/operations/models/ListTableTagsRequest/
[GetTableTagVersion]: https://lance.org/format/namespace/operations/models/GetTableTagVersionRequest/
[ListTableBranches]: https://github.com/lance-format/lance-namespace/blob/main/docs/src/spec.yaml
[QueryTable]: https://lance.org/format/catalog/rest/#querytable

Every other [spec operation](https://lance.org/format/namespace/operations/#operation-list)
returns `Unsupported` (code 0, HTTP 406): data writes,
commits, tag, branch, and index writes, transactions, rename, restore,
cascading drop, and credential vending.

### Catalog routes (`/catalog/v1`)

These have no spec model. Where a spec operation covers part of the same
data, it is linked.

| Method | Path | Related spec operation | Returns |
|---|---|---|---|
| GET | `/catalog/v1/table/:id/column-storage` | [GetTableStats][GetTableStats] (`total_bytes` only, not per column) | Bytes on disk per top-level column, excluding blob payloads. |
| GET | `/catalog/v1/table/:id/storage` | | Bytes and objects by category (data, blob, index, deletion, manifest, transaction, other) for the current version and for everything under the location, plus an estimated monthly cost. |
| GET | `/catalog/v1/table/:id/fragment-measures` | [GetTableStats][GetTableStats] (`fragment_stats` covers rows per fragment only) | Per-fragment data files, data bytes, and deleted rows (min, p25, median, mean, p75, p99, max), from the manifest. |
| GET | `/catalog/v1/table/:id/timeline` | [ListTableVersions][ListTableVersions] (versions without operations) | One event per retained version, newest first (`page_token`, `limit` up to 200): operation, details, manifest summary, writer, transaction properties, tags. |
| GET | `/catalog/v1/table/:id/version/:version/files` | [DescribeTableVersion][DescribeTableVersion] (manifest path only) | The manifest and transaction files of one version, decoded from protobuf to JSON. |
| POST | `/catalog/v1/table/:id/refresh` | | Moves the current version to the newest committed one; returns `previous_version` and `current_version`. |
| POST | `/catalog/v1/sql` | [QueryTable][QueryTable] (structured queries on one table; not implemented) | Runs one read-only SQL query (`{"sql": "...", "query_id": "..."}`) over any registered tables, each named by its full catalog path with every segment double-quoted: `"a"."b"."events"` reads the current version, `"a"."b"."events@v12"` reads version 12, and one query can join, union, or nest tables and versions. `query_id` is optional: a client-chosen id that cancels the query while it runs. Returns `tables` (`id` and `version` of each table version read, in order of first appearance), columns (`name`, Arrow `type`, field `metadata`) and rows, capped at `CATALOG_SQL_MAX_ROWS` rows and `CATALOG_SQL_MAX_RESULT_MB` of encoded rows; a capped result is flagged `truncated` with `truncated_by` `rows` or `bytes` (else `null`). `cached` is true when the per-instance result cache answered. A query starts only when the instance runs fewer than `CATALOG_SQL_MAX_CONCURRENT` and its user fewer than `CATALOG_SQL_MAX_PER_USER` queries; otherwise it is Throttling (429) at once, with the message "the server is running its maximum of N queries; try again shortly" or "you already have N queries running; wait for one or cancel it". A cancelled query answers InvalidInput (400) "query cancelled". An unregistered path or deleted dataset is TableNotFound (404) and a missing pinned version TableVersionNotFound (404), each naming the path as written. |
| POST | `/catalog/v1/sql/:query_id/cancel` | | Stops the caller's own running query `query_id` and frees its slot: 204. TransactionNotFound (404, code 10) when the caller runs no such query on this instance. |
| GET | `/catalog/v1/media/presign?uri=s3://...` | | Presigned GET URLs (`url`, `download_url`, `expires_in_secs`) for an object under `CATALOG_MEDIA_LOCATIONS` (else PermissionDenied, 403). |

### Health

| Method | Path | Returns |
|---|---|---|
| GET | `/healthz` | 200 while the process runs. |
| GET | `/readyz` | 200 once Postgres answers, else 503. |

The service has no authentication of its own; the platform's IAP guards
deployed access. The SQL routes identify the user by IAP's
`X-Goog-Authenticated-User-Email` header (its `accounts.google.com:` prefix
removed); requests without it share the user `anonymous`.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string. |
| `CATALOG_DB_SCHEMA` | unset | Schema for catalog tables and migrations; created if missing. |
| `PORT` / `CATALOG_BIND_ADDR` | unset / `0.0.0.0:8080` | Listen address; `PORT` (Cloud Run) wins. |
| `CATALOG_WEBUI_DIR` | `frontend/dist` | Built UI; without an `index.html`, only the API is served. |
| `CATALOG_VERTICALS` | unset | Comma-separated top-level namespaces allowed (deployed: `automotive,trucking,mining,robotics`). Unset allows any. |
| `CATALOG_TABLE_LOCATIONS` | unset | Comma-separated URI prefixes (e.g. `s3://bucket/lance/`) that registered table locations must fall under, matched by whole path segments. Unset allows any; tables registered earlier stay readable. |
| `CATALOG_MEDIA_LOCATIONS` | unset | Comma-separated URI prefixes that presigned media objects must fall under. Unset allows any object the storage credentials can read. |
| `CATALOG_SECRET_PREFIX` | `K_SERVICE` | Secret Manager prefix for AWS credentials on Cloud Run. |
| `CATALOG_STORAGE_PRICE_PER_GB_MONTH` | `0.0133875` | USD per GB-month for the cost estimate. |
| `CATALOG_STORAGE_PRICE_TIER` | `OCI Object Storage Active (contracted)` | Label shown with the estimate. |
| `CATALOG_SQL_MAX_ROWS` | `1000` | Rows a query returns before truncation. |
| `CATALOG_SQL_MAX_RESULT_MB` | `8` | Encoded rows a query returns before truncation. |
| `CATALOG_SQL_TIMEOUT_SECS` | `300` | Per-query time limit. |
| `CATALOG_SQL_MEMORY_BUDGET_MB` | `1024` | Memory of all running queries together (deployed: `3072`); each query's pool is this divided by `CATALOG_SQL_MAX_CONCURRENT`. Untracked memory (scans, results) comes on top. |
| `CATALOG_SQL_MAX_CONCURRENT` | `2` | Queries running at once per instance (deployed: `10`); more get 429. |
| `CATALOG_SQL_MAX_PER_USER` | `2` | Queries one user runs at once per instance; more get 429. |
| `CATALOG_SQL_THREADS` | `2` | Worker threads of the runtime queries run on, apart from request handling (deployed: `3`). |
| `CATALOG_SQL_CACHE_MB` | `64` | Query result cache per instance; bounds the estimated resident memory of cached results, not their encoded JSON size. |
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
