---
title: Table Timeline and Versions Tabs - Plan
type: feat
date: 2026-11-10
topic: table-timeline
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Table Timeline and Versions Tabs - Plan

## Goal Capsule

- **Objective:** Operators can see what happened to a table over time. That means which operations ran (appends, deletes, updates, overwrites, compaction, index builds, schema and config changes, restores), when they ran, what each one changed, and which library wrote each version. The history comes from the table's own manifest and transaction files.
- **Means:** A read-only catalog route returns one event per version, built from Lance transaction and manifest metadata. A vertical Timeline tab shows the events. The version table and trends chart move to their own Versions tab.
- **Product authority:** This contract. It supersedes the earlier request to merge Trends into a renamed Timeline tab.
- **Open blockers:** None.

---

## Product Contract

### Summary

The table page has six tabs: Overview, Properties, Timeline, Versions, Schema, and Indices.

- **Timeline:** a professional vertical event timeline, newest first. Each entry shows a version's operation as a colored, labeled marker, its commit time, and the change in rows, data size, and fragments. It also shows the operation's own details, the writer library version, any transaction properties, and the tags that point at that version. Gaps in version numbers appear as entries for versions removed by cleanup.
- **Versions:** the Trends chart above the paged version table, as the Timeline tab shows them today.

### Requirements

- R47. Each Timeline entry is one table version, newest first. It shows the operation name as a labeled, color-coded marker; the commit time (relative, with absolute on hover, following the timezone preference); and the version number.
- R48. Each entry shows the change against the previous version in rows, data size, and fragments, plus deleted rows when they changed, all from manifest summaries.
- R49. Each entry shows the operation's own details where Lance records them:
  - Append: fragments, rows, and files added.
  - Delete: the predicate, and fragments updated and removed.
  - Update: fragments updated, removed, and added, and the modified fields.
  - Overwrite: fragments, rows, and field count.
  - CreateIndex: each added index (name, columns, type) and each removed index.
  - Rewrite (compaction): rewrite groups, fragments before and after, and rewritten indices.
  - Merge (add columns): fragments and field count.
  - Project (schema change): the resulting fields.
  - UpdateConfig: the config, table-metadata, schema-metadata, and field-metadata keys that were set or removed, with the new values for config and table metadata.
  - Restore: the restored version.
  - DataReplacement: the replacement count.
  - Every other operation shows its name only.
- R50. Each entry shows the writer library name and version from the manifest, the transaction's properties when present, and the tags that point at the version.
- R51. A jump in version numbers between consecutive entries shows a cleanup marker stating which versions are no longer retained. Lance does not commit cleanup as a version, so the time of the cleanup is unknown and is not shown.
- R52. The Timeline loads a page of entries at a time (50 by default) with "Load earlier". The table can be filtered by operation type, and a legend shows each type's count among the loaded entries.
- R53. The Versions tab shows the Trends chart and the paged version table, unchanged.
- R54. The tab order is Overview, Properties, Timeline, Versions, Schema, Indices.
- R55. Building the timeline only reads metadata. It never writes to the dataset.

### Key Decisions

- **Read Lance's transaction per version** (`Dataset::read_transaction_by_version`). It is the native record of each commit's operation and details. Cleanup is never a commit, so it can only be inferred from version gaps. (user-directed: richer timeline from manifest and metadata files)
- **Vertical timeline, separate from the version table.** (user-directed)
- **Page through the timeline rather than loading it all.** Reading every transaction took 82 s for 289 versions with sequential pylance calls. A table can have thousands of versions.

### Acceptance Examples

- AE15. The prod table (60 versions) shows Append, Overwrite, CreateIndex (with index names and columns), and UpdateConfig entries. UpdateConfig entries show the changed table-metadata keys, for example `scenario_dataset_config_version`.
- AE16. `abc_130k` (289 versions) shows its Delete entry with its predicate, and pages with "Load earlier".
- AE17. Filtering to CreateIndex shows only index-build entries.

---

## Planning Contract

### Key Technical Decisions

- **KTD16. `GET /catalog/v1/table/:id/timeline?page_token&limit`**, newest first. `limit` defaults to 50 and caps at 200. `page_token` is the last version returned. The response is `{events:[...], page_token?}`. Each event has:
  - `version` and `timestamp_millis`.
  - `operation` (the Lance operation name).
  - `summary`: the manifest summary keys.
  - `writer`: `{library, version}` or null.
  - `transaction`: `{uuid, read_version, properties}` or null.
  - `tags`: names.
  - `details`: operation-specific, as in R49, with field ids resolved to names through that version's schema.

  Transactions and manifests are read up to 16 at a time per page. A version with no transaction reports `operation: "Unknown"`. A missing dataset returns 404 code 4.
- **KTD17. The Timeline UI follows the route's `page_token` through "Load earlier"** and computes deltas and cleanup gaps across loaded entries. The deltas of the last loaded entry use the next page's first entry when that page is loaded, and show "—" otherwise.

### Implementation Units

- U10. **Timeline route** (`catalog-store` and `catalog-api`). Tests use the local fixture with Append, CreateIndex, Delete, and UpdateConfig commits. They check operations, details, deltas inputs, tags, pagination, and the 404.
- U11. **UI.** The Timeline tab (vertical design, filters, legend, load more), the Versions tab (Trends plus the table), and the tab order.
- U12. **Docs.** README, ARCHITECTURE, and FRONTEND.

### Verification Contract

- Rust fmt, clippy, and tests with the database env set. The frontend build and tests pass.
- A browser check on the prod table and `abc_130k` covers AE15 through AE17, and an unchanged-prefix check covers R55.
