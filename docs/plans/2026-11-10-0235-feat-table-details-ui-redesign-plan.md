---
title: Catalog-Grade Table Details and UI Redesign - Plan
type: feat
date: 2026-11-10
topic: table-details-ui-redesign
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Catalog-Grade Table Details and UI Redesign - Plan

## Goal Capsule

- **Objective:** Operators who open a table in the catalog see the information they expect from a data catalog (Glue, HMS, Gravitino). That includes identity, ownership, registration and update times, format, schema, properties, statistics, version history, indices, and storage breakdown charts. The UI is redesigned to a professional standard.
- **Means:** Serve Lance-derived metadata through the matching read-only Lance namespace spec routes, plus one catalog route for per-column storage. Record update time and description in the catalog. Rebuild the UI around a namespace sidebar and a tabbed table page.
- **Product authority:** This Product Contract. Where it adopts them, the Lance Namespace and REST Catalog specifications govern wire behavior. The earlier Postgres catalog plan (`docs/plans/2026-11-10-0007-feat-postgres-lance-rest-catalog-plan.md`) still governs everything not changed here.
- **Open blockers:** None.

---

## Product Contract

### Summary

The table page becomes a catalog-grade view, and the catalog stays read-only against the data. Version history, table stats, indices, tags, and branches come through the spec's read-only routes. Per-column storage comes through one catalog route. The catalog records registration and update times plus an optional owner and description. The whole UI moves to a left namespace sidebar with search, and the table page gets tabs, a details sidebar, and storage charts.

### Problem Frame

The table page shows only location, owner, version, fragment count, deleted rows, properties, and schema. An operator can't tell from it how many rows or bytes the table holds, when it was registered or last changed, how it grew across versions, which columns dominate storage, or which indices exist. Those are the facts a catalog such as Glue, HMS, or Gravitino shows first. The Lance manifest already holds most of them cheaply, but the catalog marks the spec routes that expose them as Unsupported.

The interface is a functional prototype. It uses a top bar, card stacks, and namespace-by-namespace navigation, which does not scale to deep namespace trees and does not read as a professional internal tool.

### Actors

- A1. **Operator:** browses namespaces and tables, registers and deregisters tables, and inspects table details in the UI.
- A2. **Lance namespace client:** reads the same table metadata through the spec's read-only routes.
- A3. **Catalog service:** reads registered datasets on demand and never writes to them.

### Key Decisions

- **Expose Lance metadata through the spec's read-only routes.** Table stats, version list and describe, index list and index stats, tag list, and branch list become supported. The UI and Lance clients therefore share one data source. This narrows the earlier "everything else is Unsupported" rule for these read-only operations only. Governs R1-R5. (session-settled: user-directed — chosen over enriching only the describe response or adding a catalog-specific UI endpoint: one data source for UI and clients)
- **Serve per-column storage through one catalog route outside the spec.** No spec model carries per-column bytes. The route stays apart from `/v1`, and the UI calls it after the rest of the page renders. Governs R6. (session-settled: user-directed — chosen over dropping the chart or overloading the stats response's free-form context map: keeps spec responses unsurprising and fast)
- **Chart three storage views.** The charts show bytes per column, the split of storage between data, deletion, and index files, and rows and size across versions. Governs R12-R14. (session-settled: user-directed — chosen from the offered set; all three selected)
- **Record catalog-owned facts.** The catalog stores registered-at and updated-at times automatically, plus an optional owner and description set at registration. Editing after registration is deferred. Governs R7-R9. (session-settled: user-directed — chosen over adding in-UI editing or keeping only location and properties)
- **Table lists show catalog-recorded fields only.** Lists include name, owner, description, location, and registered and updated times, and never open datasets. Governs R10. (session-settled: user-directed — chosen over lazily loading rows, size, and version per list row: fast at any namespace size)
- **Table page uses a tabbed main area with a fixed details sidebar.** The tabs are Overview, Schema, Versions, and Indices, and Overview carries the charts. Governs R11-R15. (session-settled: user-directed — chosen over a Glue-style header-plus-tabs page or a single scrolling dashboard, after visual sketches)
- **App shell is a left sidebar with a namespace tree and global search.** Governs R16-R17. (session-settled: user-directed — chosen over a refined top bar with breadcrumbs or a sidebar plus a catalog home dashboard)
- **Read Lance details on demand, never cached.** This continues the earlier catalog decision. Per-column storage therefore costs its full read on every visit. Governs R6, R12.
- **Restyle on the existing component library.** "Professional" means a cohesive, consistent redesign with light and dark themes and no new brand palette. Governs R18.

### Requirements

**Spec metadata operations**

- R1. Table stats returns total bytes, row count, index count, and fragment statistics (fragment count, small-fragment count, and min, max, and mean fragment length), following the spec model.
- R2. Listing a table's versions returns each version's number, timestamp, and manifest summary (rows, files, deletions, bytes). It supports pagination, and describing a single version returns the same facts.
- R3. Listing a table's indices returns each index's name, columns, type, size, and indexed and unindexed row counts. Index stats returns the spec's index statistics.
- R4. Listing tags and branches returns the table's tags and branches, which may be empty.
- R5. Detailed describe additionally returns the current version's manifest summary, the Lance table configuration, and the dataset's storage format version.

**Catalog route**

- R6. A read-only catalog route returns bytes on disk per top-level column for the current version.

**Catalog-recorded facts**

- R7. Every table carries a registered-at time and an updated-at time. Overwrite re-registration advances updated-at, and existing tables keep their recorded registration time as their initial updated-at.
- R8. Registration accepts an optional owner and description, stored as table properties.
- R9. Catalog-recorded times are visible through describe and cannot be set or overridden by clients.

**Listing and navigation**

- R10. Namespace table lists show each table's name, owner, description, location, and registered and updated times without opening any dataset.
- R16. A left sidebar shows the namespace tree. It expands lazily, lists tables as leaves, and highlights the current location.
- R17. Global search finds namespaces and tables by name across the catalog and navigates to the match.

**Table page**

- R11. The details sidebar always shows description, owner, location (copyable), storage format, current version, registered and updated times, catalog properties, Lance table configuration, and tags and branches.
- R12. The Overview tab shows headline figures (rows, size, data files, deletions, versions, indices) and the three storage charts. The per-column chart loads after the rest of the page.
- R13. The Versions tab lists versions with timestamp and per-version rows, size, and file counts. The growth chart plots rows and size across all versions.
- R14. The Indices tab lists indices with type, columns, size, and coverage (indexed vs unindexed rows).
- R15. The Schema tab shows nested fields with type, nullability, field metadata, and schema-level metadata.

**Behavior and look**

- R18. Every page, including namespace pages, registration, and confirmations, uses one consistent professional visual language in light and dark themes.
- R19. When a dataset cannot be opened, the details sidebar and catalog-recorded facts still render, and only the Lance-derived sections show an error.
- R20. Empty sections (no indices, tags, branches, or configuration) render an explicit empty state rather than disappearing.
- R21. The catalog never writes to a registered dataset.

### Acceptance Examples

- AE1. **Covers R1, R2, R5, R12.** Given the prod table `scenario_dataset_export.1stage_scenario_dataset_v2.2026-11-07_06-46-14`, when an operator opens it, Overview shows 8,047,929 rows, about 990 MB, 4,486 data files, 39 versions, and its indices.
- AE2. **Covers R6, R12.** Given the same table, the per-column chart shows the top columns by bytes on disk after the rest of the page has rendered.
- AE3. **Covers R7, R9.** Given a client registration whose properties include a catalog-recorded time key, registration is rejected as invalid input. A normal overwrite registration advances updated-at and leaves registered-at unchanged.
- AE4. **Covers R19.** Given a registered table whose dataset files were removed, the table page still shows the sidebar facts, and the Lance-derived tabs show a not-found error.
- AE5. **Covers R13.** Given a table with 39 versions, the Versions tab pages through all 39, and the growth chart plots 39 points.
- AE6. **Covers R21.** After browsing every tab of the prod table, no object under its dataset prefix is created or modified.

### Success Criteria

- An operator answers "how big is it, when did it last change, who owns it, what's in it, and where does the storage go" from the table page without leaving the catalog.
- A Lance namespace client gets table stats, versions, and indices through the spec routes with spec-shaped responses.

### Scope Boundaries

**Deferred for later**

- Editing owner, description, or properties after registration.
- Row previews, sampling, and querying.
- Search over columns, properties, or descriptions.
- Caching derived stats.

**Outside this product's identity**

- Any write to a registered dataset.

### Dependencies / Assumptions

- The catalog's object-store credentials can read every registered dataset.
- Per-column storage takes about 11 s on the prod table (4,486 fragments) and grows with fragment count.

### Sources / Research

- Prod table inspected read-only: 39 versions with manifest summaries, Bitmap indices, Lance file format 2.2, empty table config, no tags.
- Upstream namespace impl semantics: `lance-namespace-impls` 14.0.2 `dir/namespace.rs` (`get_table_stats` uses field-level data statistics and per-fragment row counts) and `rest_adapter.rs` (route paths).
- Spec models: `lance-namespace-reqwest-client` 0.13.0 (`GetTableStatsResponse`, `TableVersion`, `IndexContent`).
- Current registration columns: `catalog-store/migrations/0001_init.sql` (`created_at` exists; no `updated_at`).

---

## Planning Contract

### Key Technical Decisions

- **KTD1. Routes mirror the upstream adapter paths.**
  - Table stats: `POST /v1/table/:id/stats`.
  - Versions: `POST /v1/table/:id/version/list` and `/version/describe`.
  - Indices: `POST /v1/table/:id/index/list` and `/index/:index_name/stats`.
  - Tags and branches: `POST /v1/table/:id/tags/list` and `/branches/list`.
  - The same id parsing, error mapping, and rejection handling as the existing routes apply. Implement each as the corresponding `LanceNamespace` method on `PostgresNamespace`. Tag and branch writes, version create and delete, and index writes stay Unsupported.
- **KTD2. Stats semantics follow upstream `DirectoryNamespace::get_table_stats`.** Total bytes come from field-level data statistics, and fragment length stats from per-fragment physical rows. This route is slow, so the UI uses the cheap manifest summary for headline figures.
- **KTD3. The per-column catalog route is `GET /catalog/v1/table/:id/column-storage`.** It returns `{version, columns: [{name, bytes_on_disk}]}`, with nested field bytes summed into their top-level column. Errors use the same `{error, code}` body.
- **KTD4. Detailed describe fills `metadata`.** It carries the current version's Lance metadata map (`total_rows`, `total_files_size`, `total_fragments`, `total_data_files`, `total_deletion_files`, `total_deletion_file_rows`, and so on) as-is. Lance manifest config entries get the `config.` prefix, and the data storage version goes under `lance.data_storage_version`. Existing describe fields are unchanged.
- **KTD5. Catalog-recorded facts are first-class in storage and surface as properties on the wire.** Postgres may hold registered-at, updated-at, owner, and description in dedicated columns; the data model is adjustable to fit this design (user-approved), through additive migrations only. Describe merges them into `properties` as `owner`, `description`, `catalog.registered_at`, and `catalog.updated_at` (RFC 3339 UTC), following HMS's `transient_lastDdlTime` precedent. Registration rejects client properties with the `catalog.` prefix (InvalidInput). Existing rows backfill updated-at from their registration time.
- **KTD6. The deletion-file share uses only measured numbers.** Data and index bytes come from manifest and index sizes. Deletion storage is shown only as file and row counts when no byte size is available.
- **KTD7. UI table lists call the cheap describe per visible row with bounded concurrency.** Spec list responses carry names only. Cheap describe reads Postgres only.
- **KTD8. Charts use `recharts` through shadcn's chart component pattern.** It follows the existing shadcn and Tailwind stack, so no second design system is added.

### Implementation Units

- **U1. Store and routes** (`catalog-store/`, `catalog-api/src/rest.rs`).
  - Add additive migrations for the catalog-recorded facts, plus the property merge and validation (KTD5).
  - Extend describe metadata (KTD4).
  - Add the stats, version list and describe, index list and stats, and tag and branch list operations (KTD1-2), plus the column-storage route (KTD3).
  - Tests: extend `catalog-store/tests/namespace.rs` and `catalog-api/tests/rest.rs` with a small local dataset of two versions and one scalar index. Cover AE3 and AE4, version pagination, the index list, stats, column storage, and Unsupported tag and branch writes.
- **U2. UI redesign** (`frontend/`).
  - Sidebar shell with a lazily expanding namespace tree and global search (R16-R17).
  - Namespace page with a table list showing catalog fields (R10).
  - Table page: tabs plus details sidebar (R11-R15), the three charts (R12-R14), error and empty states (R19-R20), and a register dialog with description.
  - A consistent visual language (R18).
  - Tests: id, format, and schema helpers in `frontend/src/lib/*.test.ts`.
- **U3. Docs.** Update the README API table, ARCHITECTURE supported operations, and FRONTEND.

### Verification Contract

- `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test --workspace` pass with `CATALOG_TEST_DATABASE_URL` and `CATALOG_REQUIRE_DB_TESTS=1`.
- `bun run build` and `bun test` pass in `frontend/`.
- Browser check against the local server with the prod table registered: AE1, AE2, AE5, every tab and both themes. AE6 is checked by listing the dataset prefix's newest objects before and after.

### Definition of Done

R1-R21 and AE1-AE6 are satisfied by tests or the browser check. Docs describe the new routes. Commits are pushed to `main` and CI passes.
