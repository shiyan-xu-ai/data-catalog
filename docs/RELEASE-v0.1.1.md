# Release notes — v0.1.1

Datasets are now organized by business vertical rather than by S3 storage
layout, and the web UI is read-only.

## Vertical namespace layout

- **Vertical allowlist.** When `CATALOG_VERTICALS` is set, a request to create a
  top-level namespace outside the list returns 400 and names the allowed
  verticals. The deployed service sets `automotive,trucking,mining,robotics`, so
  tables live under `<vertical>/<domain>/<table>`. Levels below a vertical are
  not restricted. Existing top-level namespaces outside the list stay readable
  and can be dropped once empty. When the variable is unset, any name is allowed.
- **Moving tables.** `scripts/move-tables.sh` moves registered tables to new
  paths without touching their data. It keeps each table's location, owner,
  description, and client properties, then drops old namespaces once they are
  empty. Without `--apply` it only prints the calls it would make, and it is
  safe to rerun. The deploy runbook lists the moves into the vertical layout.

## UI

- The UI is read-only. Creating and dropping namespaces, and registering
  tables, now happen only through the API.

## Upgrade notes

- Moving a table re-registers it, so `catalog.registered_at` resets and the old
  path returns 404. Update clients that use the old identifiers.
- Until an existing top-level namespace outside the allowlist is moved and
  dropped, it still accepts new child namespaces and tables.
