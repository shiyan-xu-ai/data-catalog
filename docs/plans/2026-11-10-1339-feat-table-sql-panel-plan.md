---
title: Table SQL Panel and Detail-View Consolidation - Plan
type: feat
date: 2026-11-10
topic: table-sql-panel
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Table SQL Panel and Detail-View Consolidation - Plan

## Goal Capsule

- **Objective:** Operators run read-only DataFusion SQL against the table they are viewing, from a panel on the right side of the table page. The page is also consolidated: the About sidebar folds into the Properties tab, the left navigation collapses, and the Deregister button is removed from the UI.
- **Means:** A read-only catalog SQL route backed by Lance's DataFusion table provider, with result, time, memory, and concurrency limits. A SQL editor panel replaces the details sidebar.
- **Product authority:** This contract. It amends R11 and R28 of the earlier table-details plans and lifts their "querying deferred" boundary for read-only SQL only. Everything else stands.
- **Open blockers:** None.

---

## Product Contract

### Summary

The right column of the table page becomes a SQL panel. It has an editor pre-filled with a query against the current table, a Run action (also Ctrl/Cmd+Enter), and a results grid with column types, row count, elapsed time, and a truncation notice. Queries run on the server through DataFusion against the current version, read-only and bounded. The About facts (description, owner, location, format, catalog times, tags, branches) move to the top of the Properties tab. The left navigation collapses to an icon rail, and the UI no longer offers deregistration. The API still supports it.

### Requirements

- R31. The SQL panel queries only the table being viewed. The table is addressable by its table name (quoted as needed), and the editor opens pre-filled with a valid query that selects a small sample.
- R41. Queries can time-travel. A version picker in the panel (default: latest) sets which version the plain table name reads. Any version of the same table is also addressable in SQL as `"<name>@v<N>"`, so one query can compare versions. An unknown version returns a version-not-found error.
- R42. The panel is labeled read-only and does not name the engine.
- R32. Only read-only queries run. DDL, DML, `COPY`, and other statements are rejected with an invalid-input error before execution.
- R33. Every query has limits: at most 1,000 result rows (larger results are truncated and flagged), a server-side time limit, a memory limit, and a cap on concurrent queries per instance. A query over any limit returns a clear error, never a partial silent result.
- R34. Results show column names and types, rows (nested values rendered compactly), row count, elapsed time, and whether results were truncated. Errors show the DataFusion message.
- R35. Blob columns never return payload bytes in query results.
- R36. The SQL panel docks at the bottom of the table page, spanning the full width to the right of the left navigation. It can be collapsed and resized vertically, and the main tabs keep working while a query runs.
- R43. Each Overview tile with qualifying detail carries a help (?) icon whose tooltip explains what the figure covers and excludes. Every help icon in the UI opens on hover and keyboard focus.
- R37. The Properties tab starts with the former About facts (description, owner, copyable location, registered and updated times, tags, branches), followed by catalog properties and Lance table configuration. Each fact appears once in the table view: storage format and current version live in the page header, schema metadata lives in the Schema tab, and object counts live in tab badges. Overview tiles do not repeat them.
- R38. The left navigation collapses to an icon rail and expands again, remembering the choice across reloads.
- R39. The UI shows no Deregister action. The deregister API remains.
- R40. The SQL route never writes to the dataset, the catalog database, or the local filesystem.
- R44. Overview shows the Lance-native table stats (`GetTableStats`) that no other tile already shows, charted. That means fragment statistics: the rows-per-fragment distribution (min, p25, p50, p75, p99, max, mean) and the share of small fragments below Lance's 1,048,576-row compaction target. A help icon explains where the figures come from. The field-statistics `total_bytes` appears in the Storage by column tooltip next to the manifest data size, not as a new tile.
- R45. Storage by column carries a help icon. The tooltip says it shows bytes on disk per top-level column in the current version's data files (field-level data statistics). It excludes blob payload files, which hold most of the storage cost, and index files. Blob columns count only their per-row descriptors (pointers into `.blob` files).
- R46. The growth chart section is titled "Trends".

### Key Decisions

- **SQL engine: DataFusion over Lance's table provider.** This is what the user asked for; other engines are deferred. The UI does not name the engine.
- **Time travel by version number.** Lance checks out any retained version by number, tag, or branch (`DatasetBuilder::with_version` / `with_tag` / `with_branch`, `Dataset::checkout_version`). It has no as-of-timestamp API in Rust; pylance resolves `asof` by scanning version timestamps. The panel's picker shows each version's timestamp, so picking by time is a UI concern. Versioned names resolve lazily, so only the versions a query references are opened. (user-directed: chosen over current-version-only)
- **Catalog route outside the spec.** The spec's query operation is a vector/filter request, not SQL, so the SQL route sits with the other catalog routes.
- **Read-only by construction.** Use DataFusion's SQL options with DDL, DML, and statements disabled, plus no registered object stores for writing.
- **Bounded by default.** Prod tables hold millions of rows and terabytes of blob payloads, and the service runs on 1 GiB of memory.

### Scope Boundaries

- Deferred: query history and saved queries, other engines, cross-table joins, version time travel, result export.
- Outside identity: any write through SQL.

### Acceptance Examples

- AE10. On the prod table, the pre-filled query returns rows within a few seconds, and the grid shows types, row count, and elapsed time.
- AE11. `SELECT count(*) FROM <table>` returns 8,097,329 (or the current row count), matching the Overview figure.
- AE12. `CREATE TABLE`, `INSERT`, `COPY ... TO`, and `DROP` are each rejected with an invalid-input error, and nothing is written.
- AE13. A query returning more than 1,000 rows shows 1,000 rows and a truncation notice.
- AE14. The Properties tab shows the location and catalog times, there is no right-hand About card, the left navigation collapses and stays collapsed after a reload, and no Deregister button appears.

---

## Planning Contract

### Key Technical Decisions

- **KTD12. `POST /catalog/v1/table/:id/sql`.**
  - Body: `{sql}`.
  - Response: `{columns: [{name, type}], rows: [[...]], row_count, truncated, elapsed_ms}`. Rows are JSON arrays of arrow values, converted with Arrow's JSON writer semantics.
  - Errors use `{error, code}`: invalid SQL or a rejected statement is 400 code 13, an exceeded limit is 400 code 13 with a clear message, a missing dataset is 404 code 4, and too many concurrent queries is 429 code 21 (Throttling).
- **KTD13. Execution.**
  - Build a fresh `SessionContext` per query with a bounded memory pool. Register the current dataset through `lance::datafusion::LanceTableProvider` under the table's leaf name.
  - Plan the query with `sql_with_options` (DDL, DML, and statements disallowed), then wrap the plan in a limit of 1,001 rows to detect truncation.
  - Run under `tokio::time::timeout`, behind a per-instance semaphore.
  - Limits are config: `CATALOG_SQL_MAX_ROWS` (1000), `CATALOG_SQL_TIMEOUT_SECS` (60), `CATALOG_SQL_MEMORY_LIMIT_MB` (512), `CATALOG_SQL_MAX_CONCURRENT` (2).
- **KTD14. Blob columns.** Prefer projecting blob columns as their descriptor (or excluding them from `*`) so payload bytes are never read. Pick whichever Lance 14 supports and record it in the report.
- **KTD15. Editor.** Use CodeMirror 6 with SQL highlighting, wrapped as a shadcn-styled component. Results render in a virtualized grid only if needed; 1,000 rows renders fine as a plain table with sticky headers.

### Implementation Units

- **U7. SQL route** (`catalog-store` or `catalog-api`, wherever the provider and limits fit best). Add the route, config, and limits. Tests use the local fixture: a select with types, the truncation flag, rejected DDL, DML, and `COPY`, and a missing dataset.
- **U8. UI** (`frontend/`). Add the SQL panel (R31, R34, R36), Properties consolidation (R37), collapsible navigation (R38), and removal of Deregister (R39). Helper tests cover query pre-fill quoting and result value formatting.
- **U9. Docs.** Update the README API and env tables, and ARCHITECTURE.

### Verification Contract

- Rust fmt, clippy, and tests with the database env set; frontend build and tests.
- Browser check on the prod table covering AE10, AE11, AE13, and AE14, plus AE12 through the UI. An unchanged-prefix listing before and after shows R40.
