# AGENTS.md

This repo is a Lance data catalog service: an S3-native metadata catalog for
Lance tables, written in Rust (axum + tower, `lance` crate, `object_store`,
`kube-rs`).

- Full long-term design: `docs/design.md`.
- Current implementation plan (v1.0.0 scope): `.planning/data-catalog-v1/`
  (`task_plan.md` for phases, `findings.md` for locked scope decisions).

## Workspace layout

Cargo workspace with three crates:

- `catalog-core` — shared types and the registry model.
- `catalog-store` — object-store IO and S3 sweep/TTL logic.
- `catalog-api` — the `catalog-api` binary: axum HTTP server, sweep loop,
  TTL engine, leader election.

## Conventions

- Rust edition 2021, toolchain pinned in `rust-toolchain.toml`.
- `cargo build --workspace` and `cargo test --workspace` must pass before
  committing.
- 4-space indentation, LF line endings, UTF-8 (see `.editorconfig`).
- Persisted types in `catalog-core/src/types.rs` are stored as JSON (registry
  and TTL-audit columns) and read across binary versions during rolling
  upgrades. Every new persisted field MUST be added with `#[serde(default)]`
  (and a `Default`-able type) so an older row that omits it still deserializes;
  identity/structural fields (ids, timestamps, enums) stay required.

## Commit messages

Commit subjects (and PR titles — PRs are squash-merged, so the PR title
becomes the final commit message) must follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>)?: <description>
```

Types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
`revert`, `style`, `test`. Example: `fix(api): return 404 for unknown table id`.

This is enforced in CI (`.github/workflows/commit-lint.yml`) against both the
PR title and every commit in the PR. Enable the matching local pre-commit
check with:

```
git config core.hooksPath .githooks
```
