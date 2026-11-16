# AGENTS.md

A Lance REST catalog service: Rust (axum, sqlx, `lance`, `lance-namespace`) with a
React UI, shipped as one container on Apps Platform (Cloud Run). Postgres stores
namespaces and table registrations; Lance manifests stay authoritative for table
contents. The catalog never writes to a registered dataset.

## Where things are

| Path | What |
|---|---|
| `catalog-store/` | `PostgresNamespace` (spec implementation), catalog-route logic (`storage.rs`, `timeline.rs`, `sql.rs`, `media.rs`), migrations, vendored Lance protos. |
| `catalog-api/` | The binary: config (`config.rs`), routes (`rest.rs`), Secret Manager loading, UI serving. |
| `catalog-*/tests/` | Integration tests; Postgres-backed ones share `catalog-store/tests/support/`. |
| `frontend/` | React UI; see `docs/FRONTEND.md`. |
| `scripts/move-tables.sh` | Moves registrations between catalog paths. |
| `README.md` | Architecture diagram, every API route, configuration, local setup. |
| `ARCHITECTURE.md` | Data model, current version, caching, SQL, media, deployment. |
| `docs/RUNBOOK-DEPLOY.md` | Deploying and operating. |
| `docs/RELEASE-v*.md`, `docs/plans/` | Release notes and past plans; history, not instructions. |

## Commands

```sh
# Local Postgres for tests and the server
docker run -d --name catalog-pg -e POSTGRES_PASSWORD=pg -p 55432:5432 postgres:17
export DATABASE_URL=postgres://postgres:pg@127.0.0.1:55432/postgres

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
CATALOG_TEST_DATABASE_URL=$DATABASE_URL CATALOG_REQUIRE_DB_TESTS=1 cargo test --workspace --all-targets
cargo run --bin catalog-api                     # serves frontend/dist at :8080

cd frontend && bun install && bun run build && bun run test
```

CI runs all of these except the frontend tests (`bun run test`), so run those
locally. Without `CATALOG_TEST_DATABASE_URL`, Postgres-backed tests skip
silently; `CATALOG_REQUIRE_DB_TESTS=1` makes skipping fail, as CI does.
Building needs `protoc`.

## Rules

- **Read-only toward data.** Never add code that writes, deletes, or commits to
  a registered dataset or a media object. Refresh and registration write only
  Postgres.
- **Migrations are append-only.** Schema changes are new files in
  `catalog-store/migrations/`; never edit an applied one.
- **Current version.** Reads without a version use the table row's
  `current_version`. Moving it goes through registration, refresh, or the
  cleanup compare-and-set; keep those writes conditional (see
  `ARCHITECTURE.md`).
- **Keep docs in step.** A route or env var change updates the README tables
  in the same PR. A UI behavior change updates `docs/FRONTEND.md`.
- Rust 2021, toolchain pinned in `rust-toolchain.toml`. LF, UTF-8; 4-space
  indent for Rust, 2 for YAML, TOML, and JSON (`.editorconfig`).

## Pull requests and releases

- `main` takes changes only through pull requests, squash-merged, so the PR
  title becomes the commit message. Titles must follow
  [Conventional Commits](https://www.conventionalcommits.org/)
  (`<type>(<scope>)?: <description>`, types `build`, `chore`, `ci`, `docs`,
  `feat`, `fix`, `perf`, `refactor`, `revert`, `style`, `test`); CI checks it
  (`.github/workflows/commit-lint.yml`). `git config core.hooksPath .githooks`
  checks commit subjects locally.
- A release is a PR titled `chore(release): set version X.Y.Z` that sets the
  version in `Cargo.toml` (workspace, plus `Cargo.lock`) and
  `frontend/package.json` and adds `docs/RELEASE-vX.Y.Z.md`. After it merges,
  tag `vX.Y.Z` on the merge commit and publish a GitHub release with that file
  as the notes.
