# pp — Printing Press (Rust)

OpenAPI 3.0 → installable Rust CLI generator. Backed by [`cargo-progenitor`](https://crates.io/crates/cargo-progenitor) (codegen) + minijinja templates (wrapper layer).

> Status: pre-alpha. Week 1 scaffold only. `inspect` works; `generate` and `validate` are stubs.

## Prerequisites

```bash
cargo install cargo-progenitor
```

## Usage (planned)

```bash
pp inspect spec.yaml                            # print derived facts as JSON
pp generate spec.yaml -o ./out --build          # produce a cargo workspace
```

See plan: `../docs/plans/2026-05-12-001-feat-rust-printing-press-mvp-plan.md`.
