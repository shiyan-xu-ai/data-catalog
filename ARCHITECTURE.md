# Architecture

The overview diagram, routes, and configuration are in the [README](README.md).
This file covers how the pieces work.

## Crates

- `catalog-store`: `PostgresNamespace`, the `lance_namespace::LanceNamespace`
  implementation. Also holds the catalog routes' logic (storage, timeline, SQL,
  media presigning), the sqlx migrations in `catalog-store/migrations/`, and
  protos vendored from `lance-table` in `catalog-store/protos/` for decoding a
  version's files.
- `catalog-api`: the `catalog-api` binary. Config, Secret Manager credential
  loading, the `/v1` and `/catalog/v1` routers, health routes, and static UI
  serving.

## Data model

- `namespaces(path text[] primary key, parent_path -> namespaces ON DELETE RESTRICT, properties)`.
  The root namespace is implicit and has no row.
- `tables(namespace_path -> namespaces ON DELETE RESTRICT, name, location, properties,
  owner, description, current_version, current_e_tag, registered_at, updated_at)`,
  primary key `(namespace_path, name)`.

The foreign keys make "parent must exist" and "drop only when empty" hold under
concurrent requests. On the wire, `owner` and `description` are ordinary table
properties, and the timestamps appear as the server-managed
`catalog.registered_at` and `catalog.updated_at` properties; clients cannot set
`catalog.*` keys. Other properties, such as `business_owner` and
`technical_owner`, stay in the `properties` JSON.

## Request handling

The upstream `RestAdapter` keeps its router private, so `catalog-api` has its
own axum router that follows the adapter's conventions: `$`-joined path ids, a
body `id` that disagrees with the path is invalid input, and spec error codes
map to HTTP status (not found 404, exists or not empty 409, invalid 400,
unsupported 406, internal 500). Unmatched routes under `/v1` and `/catalog/v1`
return `Unsupported`.

## Current version and caching

Every read that names no version, on both routers, answers for the table's
current version, which the table row stores with its manifest e-tag. Every
instance reads it from the row, so all instances answer with the same version.

- **Registration** records the newest version. A commit made directly to Lance
  does not become current on its own.
- **Refresh** writes the row only when it moves the version forward (one
  conditional `UPDATE`). Concurrent refreshes of a table make one write, and a
  slower one never moves the version back. A dataset recreated at the same
  location with lower version numbers needs re-registration.
- **Cleanup.** When the current version's files are gone, the next read that
  needs them advances to the newest version with a compare-and-set. A dataset
  with no manifest left returns `TableNotFound` (404).
- Version lists, the timeline, tags, and branches always show what the dataset
  holds, including versions newer than the current one.

A Lance version never changes, so everything derived from one is cached per
instance in the Lance session, keyed by location, version, and manifest e-tag
(the e-tag keeps a dataset recreated at the same location from reusing stale
entries): opened datasets, data statistics, fragment row counts, deleted-row
counts, and the storage listing. A warm instance answers repeat reads without
touching object storage. `CATALOG_CACHE_MEMORY_LIMIT_MB` bounds the session,
three quarters for metadata and the rest for index metadata.

## SQL

A query names tables by their full catalog path (`"a"."b"."events"`, optionally
`"a"."b"."events@v12"`). The server parses the statement, opens only the tables
it names, and gives each query a fresh DataFusion session that holds exactly
those tables: a plain path reads the table's current version, and a pinned path
reads that version. DDL, DML, statements, and spilling are disabled, so a query
cannot write anywhere. Blob columns read as their descriptors (`kind`,
`position`, `size`, `blob_id`, `blob_uri`), so payload bytes are never fetched.
Row, byte, time, and memory limits bound each query.

Queries run on a Tokio runtime of their own (`CATALOG_SQL_THREADS` workers),
so plan and execution work never occupies the threads that answer catalog
requests. Lance decodes on its own CPU pool, which browsing shares.

**Admission.** One lock-guarded registry tracks every running query with its
user and optional client id. A query starts only when fewer than
`CATALOG_SQL_MAX_CONCURRENT` run on the instance and fewer than
`CATALOG_SQL_MAX_PER_USER` run for its user; otherwise it is rejected at once
(429), with no queue. The user is IAP's `X-Goog-Authenticated-User-Email`
header, or `anonymous` without it; every admission decision logs it. A query
leaves the registry, freeing its slot, when it finishes, when its owner cancels
it by id, or when its request is dropped (client disconnect), and the last two
abort its task. Slots and the registry are per instance, so with several
instances a cancel can land where the query is not running (404).

**Memory.** `CATALOG_SQL_MEMORY_BUDGET_MB` is split evenly across the slots into
each query's DataFusion pool. The pool tracks large operators (sorts, joins,
group-bys), not scan buffers or the result being built, so the budget sits well
below the instance memory. A query over its pool fails with a clear error.

**Result size.** Rows are streamed and encoded until `CATALOG_SQL_MAX_ROWS` or
`CATALOG_SQL_MAX_RESULT_MB` would be exceeded; the response's `truncated_by`
names the cap.

**Result cache.** Completed results are cached per instance in memory, shared
by all users, and evicted least recently used first once their estimated
resident memory (parsed JSON values, not encoded bytes) exceeds
`CATALOG_SQL_CACHE_MB`. The key is the trimmed query text, the row and byte
caps, and the sorted location, version, and e-tag of every table the statement
resolved to. Tables are opened (metadata only, itself cached) before the
lookup, so a hit reads no table data, and a refresh that moves a current
version changes the key. A version never changes, so a hit is never stale.
Results that depend on more than those versions are never cached: `EXPLAIN`,
statements reading no table, and plans calling a volatile or stable function
such as `random()` or `now()`. A hit still answers for a current version whose
files cleanup removed; only a query that reads the files advances it.

## Storage footprint

Lance has no API for total storage size: manifest summaries and field
statistics cover one version's data files, not blob payload files
(`data/*.blob`), index files, or files kept for older versions. The storage
route lists object metadata under the location, groups it by category, and
prices it at `CATALOG_STORAGE_PRICE_PER_GB_MONTH`.

## Media

`/catalog/v1/media/presign` signs GET URLs with the catalog's storage
credentials and makes no storage request. The browser then reads the object
directly, with byte ranges, so the bucket's CORS rules must allow the UI's
origin and the `Range` header. Only objects under `CATALOG_MEDIA_LOCATIONS` are
signed, and only locations under `CATALOG_TABLE_LOCATIONS` can be registered;
with either unset, the storage credentials are the only bound, and the server
logs a warning at startup.

## Deployment

One Cloud Run container serves the API and the UI on `$PORT`. The `Dockerfile`
builds the UI with Bun, compiles the binary, and copies the UI to `/app/webui`
(`CATALOG_WEBUI_DIR`); the server falls back to `index.html` for deep links.
`docker-entrypoint.sh` starts the Cloud SQL Auth Proxy and derives
`DATABASE_URL` and `CATALOG_DB_SCHEMA`. The server retries its first database
connection for 30 seconds, runs migrations, and reports ready once Postgres
answers. Steps are in the [deploy runbook](docs/RUNBOOK-DEPLOY.md).
