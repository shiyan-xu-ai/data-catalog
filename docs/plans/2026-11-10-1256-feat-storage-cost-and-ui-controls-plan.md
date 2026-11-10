---
title: Table Storage Footprint, Cost Estimate, and Detail-View Layout - Plan
type: feat
date: 2026-11-10
topic: storage-cost-and-ui-controls
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Table Storage Footprint, Cost Estimate, and Detail-View Layout - Plan

## Goal Capsule

- **Objective:** On the table page, operators can tell how much data the current version holds apart from how much the table occupies on object storage, and see an estimated monthly storage cost. Properties and configuration move to their own tab. Theme and timezone controls sit together at the bottom of the sidebar.
- **Means:** A read-only catalog route lists the dataset prefix and prices the total at the contracted OCI rate. The UI is updated to use it.
- **Product authority:** This contract. It amends `docs/plans/2026-11-10-0235-feat-table-details-ui-redesign-plan.md` (R11, R12) and changes nothing else.
- **Open blockers:** None.

---

## Product Contract

### Summary

The Overview tab separates current-version data from total bytes on object storage, and shows an estimated monthly storage cost based on the total. A new Properties tab, second after Overview, carries catalog properties, Lance table configuration, and schema-level metadata. The details sidebar keeps identity, format, catalog times, tags, and branches. Theme (light, dark, system) and timezone (UTC, local) toggles live together at the bottom of the left sidebar.

### Problem Frame

"Data size" shows Lance's manifest summary: the data files the current version references. On the prod table that is 990 MB. The dataset prefix holds about 19 TB, almost all of it blob v2 payload files (`data/*.blob`, 20,069 files) that the summary does not count. The page currently reads as if the table were under 2 GB, and nothing ties its footprint to what it costs to store.

Lance 14 has no public API that returns total storage size. The manifest summary and field statistics cover only the current version's data files. Lance's own tests measure on-disk size by listing the dataset directory (`lance` 14.0.2 `dataset/cleanup.rs`, `read_dir_all`).

### Requirements

- R22. Overview labels the manifest figure as current-version data and states that it excludes blob payloads, index files, and files kept only by older versions.
- R23. Overview shows total bytes on object storage under the dataset location, broken down into data files, blob files, index files, deletion files, manifests, transactions, and other.
- R24. Overview shows an estimated monthly storage cost: total bytes on storage times the configured price per GB-month. It also shows the rate, its tier, and that it is an estimate.
- R25. The storage-composition chart uses the on-storage breakdown. The per-column chart is labeled as current-version data excluding blob payloads.
- R26. The table detail tabs are, in order: Overview, Properties, Schema, Versions, Indices.
- R27. The Properties tab shows catalog properties (owner, description, client properties), Lance table configuration, and schema-level metadata, each with an explicit empty state.
- R28. The details sidebar no longer shows properties or table configuration. It keeps description, owner, location, format, catalog times, tags, and branches.
- R29. The bottom of the left sidebar holds a theme control (light, dark, system) and a timezone control (UTC, local). Both choices persist across reloads, and every displayed timestamp follows the timezone choice.
- R30. Listing for the storage footprint reads only object metadata under the dataset location and never writes.

### Key Decisions

- **Measure total storage by listing the dataset prefix on demand.** No Lance API returns it, and caching stays out of scope. On the prod table this means about 25 list requests for about 24,700 objects. It loads after the rest of the page, like per-column storage.
- **Price at the contracted OCI Active-tier rate, $0.0133875 per GB-month (decimal GB), configurable.** The source is the 2026 OCI rate-card thread linked from the storage-cost discussion. Active is 47.5% off list, Infrequent is $0.0065 and Archive is $0.00195. The estimate is an upper bound when objects have been auto-tiered to Infrequent, and the UI says so. Requests, retrieval, and egress are excluded.
- **Scope the footprint to the registered location only.** Sibling directories next to `dataset.lance` (for example `lance_tags/` or `curated_indices/`) are not part of the table.

### Acceptance Examples

- AE7. On the prod table, Overview shows current-version data of about 990 MB, labeled as such. Total on storage is about 19,069 GB, with blob files about 19,067 GB. The estimated cost is about $255 per month at $0.0133875 per GB-month.
- AE8. The Properties tab is second, and the sidebar shows no properties or configuration sections.
- AE9. Switching timezone to UTC changes the registered and updated times in the sidebar and the version timestamps. Reloading keeps the choice.

---

## Planning Contract

### Key Technical Decisions

- **KTD9. `GET /catalog/v1/table/:id/storage`** returns:
  - `version`
  - `current: {data_bytes, index_bytes}`. `data_bytes` is the manifest `total_files_size`; `index_bytes` is the sum of current index sizes.
  - `on_storage: {total_bytes, total_objects, categories: [{name, bytes, objects}]}`
  - `pricing: {price_per_gb_month, currency, tier, estimated_monthly_cost}`

  The listing goes through the dataset's object store, rooted at the registered location. Categories come from the path relative to that location: `data/*.lance` → data, `data/*.blob` → blob, `_indices/` → index, `_deletions/` → deletion, `_versions/` → manifest, `_transactions/` → transaction, otherwise other. A missing dataset returns `TableNotFound`.
- **KTD10. Price config.** `CATALOG_STORAGE_PRICE_PER_GB_MONTH` defaults to `0.0133875`, and `CATALOG_STORAGE_PRICE_TIER` defaults to `OCI Object Storage Active (contracted)`. Both are surfaced in the route response, so the UI never hard-codes a price.
- **KTD11. UI preferences** are stored in `localStorage`. Timestamps render through one shared formatter that reads the timezone preference.

### Implementation Units

- **U4. Storage route** (`catalog-store`, `catalog-api`). The route, config, and one test on the local fixture asserting category totals and cost arithmetic.
- **U5. UI** (`frontend/`). Overview storage cards and charts (R22-R25), Properties tab (R26-R27), sidebar trim (R28), sidebar-bottom preferences (R29), and helper tests for cost and size formatting.
- **U6. Docs.** README env and API tables, plus ARCHITECTURE.

### Verification Contract

- Rust fmt, clippy, and tests with the database env set; frontend build and tests.
- Browser check on the prod table covering AE7-AE9, followed by an unchanged-prefix check (R30).
