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

Each query gets a fresh DataFusion session that exposes only the addressed
table: its name reads the requested version, and `"<name>@v<N>"` opens version
N on demand. DDL, DML, statements, and spilling are disabled, so a query cannot
write anywhere. Blob columns read as their descriptors (`kind`, `position`,
`size`, `blob_id`, `blob_uri`), so payload bytes are never fetched. Row, time,
and memory limits bound each query, and a per-instance semaphore caps
concurrency.

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
origin and the `Range` header. Any object those credentials can read can be
signed, so scope the credentials accordingly.

## Deployment

One Cloud Run container serves the API and the UI on `$PORT`. The `Dockerfile`
builds the UI with Bun, compiles the binary, and copies the UI to `/app/webui`
(`CATALOG_WEBUI_DIR`); the server falls back to `index.html` for deep links.
`docker-entrypoint.sh` starts the Cloud SQL Auth Proxy and derives
`DATABASE_URL` and `CATALOG_DB_SCHEMA`. The server retries its first database
connection for 30 seconds, runs migrations, and reports ready once Postgres
answers. Steps are in the [deploy runbook](docs/RUNBOOK-DEPLOY.md).
