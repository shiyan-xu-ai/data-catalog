# Lance Data Catalog — Frontend Design Decisions

Companion to `lance-catalog-design-v3.md`. Covers the catalog web UI: stack selection, catalog-specific libraries, API integration, serving, and dev loop. Guiding constraints: sleek modern internal tool, table-centric UX, 10k+ tables, Claude Code as the primary implementation driver (favor typed, well-documented, high-ecosystem-coverage libraries), and the same one-manifest-set / spec-pinned philosophy as the backend.

---

## 1. Core stack (validated)

| Choice | Role | Rationale |
|---|---|---|
| **React + Vite + TypeScript** | App framework + build | Fastest iteration loop with Claude Code; the dominant ecosystem for data-tool UIs; Vite dev server + HMR keeps the loop in seconds |
| **Tailwind CSS** | Styling | "Sleek modern" without fighting a heavy design system; utility classes co-locate with components (agent-friendly) |
| **shadcn/ui** | UI primitives | Accessible dialog/dropdown/combobox/tooltip/command-palette primitives that are **owned and restylable** — catalogs need combobox/multi-select filters constantly; no vendor theme to fight |
| **TanStack Table** | Table/list engine | The load-bearing pick: catalogs live and die by the table view (sorting, column visibility, pagination, row expansion for schema). Headless → looks like our design, not an admin template |
| **TanStack Query** | Server state | Fetch/cache/invalidate for search + filter results; clean loading/error states; devtools; pairs with codegen'd hooks (§4) |
| **TanStack Router** *(swapped in — see §2)* | Routing + URL state | Typed, schema-validated search params; loader integration with Query |

## 2. One swap: React Router → TanStack Router

A catalog's core UI state is **search/filter/sort/pagination**, and that state must live in the URL so views are shareable and bookmarkable ("all tables in `av.perception` with unindexed rows > 0"). TanStack Router makes search params first-class: schema-validated (zod), fully type-inferred, with helpers for updating filter state without string-building. It also integrates natively with TanStack Query for route-level data loading.

React Router v7 is acceptable and more familiar; the tiebreaker is that Claude Code writes most of this code — end-to-end type inference means broken filter links and stale param names fail at compile time rather than in review.

Route sketch:
```
/tables                     # main list: search, filters, saved views (all in URL params)
/tables/$tableId            # overview: schema, health, aux tables, freshness
/tables/$tableId/versions   # version list annotated with operations (§5.10 backend)
/tables/$tableId/history    # operation log ($operations)
/tables/$tableId/indices    # index set + staleness + lifecycle events
/tables/$tableId/profile    # column distribution profiles + drift diff
/tables/$tableId/preview    # bounded data preview (QueryTable)
/tables/$tableId/lineage    # DAG view
/namespaces/$ns             # namespace browse
/policies · /costs · /jobs  # governance surfaces
```

## 3. Required additions (catalog-specific)

| Library | Why it's required here |
|---|---|
| **TanStack Virtual** | Non-negotiable at 10k+ tables: virtualized rows for the main list, and again for wide schemas (AV tables with hundreds of columns). Built to pair with TanStack Table |
| **@xyflow/react (React Flow) + elkjs** | Lineage DAG over the petgraph API: interactive nodes, expand upstream/downstream, aux tables rendered as `attached-to` children. Every serious lineage UI (DataHub, Marquez, OpenMetadata) converges on this pattern; elkjs does layered DAG layout |
| **apache-arrow (JS)** | `QueryTable`/Flight SQL return Arrow IPC — consume record batches directly in the browser (no JSON round-trip) and feed the preview grid |
| **openapi-typescript** (or **orval** for generated TanStack Query hooks) | Same philosophy as the Rust side: pinned `spec.yaml` (+ `/ext` OpenAPI) → generated types/hooks → frontend breaks at compile time on spec bumps. No hand-written API types |
| **zod** | Search-param schemas (router) + runtime validation at the API boundary |

Charts: **shadcn/ui chart components (Recharts-based)** for profile histograms, health sparklines, cost trends — stays in-family with design tokens. Operational time-series (loop lag, RED) remain **Grafana's job** (§12 of the main doc); do not rebuild ops dashboards in-app.

Data preview grid: TanStack Table is sufficient given §5.4's row/byte caps. A canvas grid (e.g. glide-data-grid) is the *escape hatch only if* interactive data exploration ever becomes a real workload — do not start there.

## 4. API integration rules

- **Generated client only.** Types/hooks from the pinned spec + `/ext` schema; CI regenerates and fails on drift — mirrors the backend's conformance stance.
- **Freshness is visible.** Every API payload carries `as_of` (snapshot staleness by design). Surface it (relative timestamp on detail views); align Query `staleTime`/`refetchInterval` with backend freshness semantics so the UI never pretends to be more current than the catalog is.
- **URL is the state of record** for list views: query, namespace, filters, sort, page cursor, saved-view id. Component state only for ephemeral UI (open panels, hover).
- **Server-driven list operations.** Sorting/filtering/pagination execute against `/ext/v1/tables` (10k tables never fully load client-side); TanStack Table runs in manual mode fed by Query.
- **Errors**: map the spec's numeric error model to typed UI states (NotFound, PermissionDenied, conflict) rather than string-matching messages.

## 5. Explicitly not chosen

| Rejected | Because |
|---|---|
| Next.js / SSR | Internal tool behind auth — no SEO/SSR case; SPA + static assets is simpler and fits the serving model (§6) |
| Redux / Zustand-by-default | Server state belongs to Query; list/filter state belongs to the URL; the remainder is `useState`. Add a store only if a concrete cross-cutting client state appears |
| MUI / Ant Design / Mantine | Component suites impose a theme you'd fight for the "sleek" goal; shadcn/Tailwind keeps ownership |
| AG Grid (as default) | Heavyweight/commercial features unneeded at preview-cap scale; TanStack Table + Virtual covers the requirement headlessly |
| Hand-rolled D3 lineage | React Flow + elkjs is the established pattern; custom D3 graphs are a maintenance sink |
| In-app ops dashboards | Grafana owns operational time-series (§12); the app shows table/business health only |

## 6. Serving & deployment

- `vite build` → static assets served by **catalog-api** via `tower-http::ServeDir` (SPA fallback to `index.html`). No new deployment, no Node in prod, same origin as the API → inherits the OIDC session and avoids CORS entirely.
- Same-origin also covers the MCP/`/ext`/spec routes; assets are fingerprinted and cache-forever.
- If the UI later needs independent release cadence, split to a static bucket + CDN — not needed at day one.

## 7. Dev loop (matches backend §11.2)

- Frontend registered as a Tilt `local_resource` running `vite dev`, with Vite's proxy pointed at the port-forwarded catalog-api Service — one `tilt up` runs backend + frontend against in-cluster MinIO/Spark.
- Codegen (`openapi-typescript`/orval) wired as a Tilt dependency on the spec files, so API changes regenerate types live.
- CI: typecheck + lint + build + a smoke Playwright pass against the kind overlay (same manifests as prod).

## 8. Dependency list (initial `package.json` surface)

```
react · react-dom · typescript · vite
tailwindcss · shadcn/ui (radix primitives) · lucide-react
@tanstack/react-table · @tanstack/react-virtual · @tanstack/react-query · @tanstack/react-router
@xyflow/react · elkjs
apache-arrow
zod · openapi-typescript (dev) — or orval (dev)
recharts (via shadcn charts)
```

Kept deliberately small; every addition beyond this list needs a stated reason in review.
