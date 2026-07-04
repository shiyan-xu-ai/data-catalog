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
