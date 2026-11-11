# Release notes — v0.1.0

First versioned release of the Lance REST catalog: a Postgres-backed catalog for
existing Lance datasets, with a web UI and read-only SQL. The supported surface
is documented in [README.md](../README.md) and
[ARCHITECTURE.md](../ARCHITECTURE.md).

## Catalog

- **Postgres metastore.** Nested namespaces and table registrations (location,
  owner, description, properties, registered and updated times) live in
  Postgres. Lance manifests stay authoritative for table state.
- **Lance REST Catalog routes.** Namespace list, describe, create, drop, and
  exists; table list, list-all, register, describe, exists, and deregister;
  read-only table stats, version list and describe, index list and stats, and
  tag and branch lists. Other spec operations return `Unsupported`.
- **Read-only by construction.** Registration opens a dataset to validate it.
  No route writes to a registered dataset.

## Catalog routes outside the spec

- `GET /catalog/v1/table/:id/column-storage`: bytes per top-level column of the
  current version.
- `GET /catalog/v1/table/:id/storage`: objects under the table location by
  category, with an estimated monthly storage cost.
- `POST /catalog/v1/table/:id/sql`: read-only DataFusion SQL with version time
  travel (`"<table>@v<N>"`) and row, time, memory, and concurrency limits.
- `GET /catalog/v1/table/:id/timeline`: per-version operation history from each
  version's Lance transaction and manifest.

## UI

- Namespace tree, global search, and theme and timezone preferences.
- Table page tabs: Overview (tiles, storage charts, fragment distribution),
  Properties, Timeline (vertical operation history with a detail pane),
  Versions (paginated history with a sticky trends chart), Schema, and Indices.
- SQL panel docked at the bottom, with a collapsible and resizable editor,
  resizable result columns, and CSV download.

## Deployment

- One Cloud Run container serves the API and UI. With `enable_postgres`, the
  entrypoint starts the Cloud SQL Auth Proxy and derives `DATABASE_URL`. See
  [the deploy runbook](RUNBOOK-DEPLOY.md).

## Known limitations

- Data operations, catalog-managed versions and commits, tag, branch, and index
  writes, and credential vending are not implemented.
- Cleanup of old versions is not committed by Lance, so the timeline shows it
  only as a gap in version numbers.
- Column-storage, storage-listing, table-stats, and version-history reads scale
  with fragment and version counts and are slow on large tables.
