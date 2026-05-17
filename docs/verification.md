# Verification profiles

`pp` separates fast source checks from generated-artifact confidence checks so normal PRs stay quick while generated CLIs and MCP servers are still exercised regularly.

## Fast

Run on every push and pull request via `.github/workflows/ci.yml`.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

Purpose:

- Rust formatting and lint health.
- Unit and integration tests that do not build generated release workspaces.
- Focused OpenAPI 3.0 and OpenAPI 3.1 safe-subset regressions for preserving supported primitive schema annotations without type inference or unsupported-keyword fallback.
- Inspect/slicing/pipeline/model/backend behavior.
- Auth-selection coverage for fail-ambiguous defaults, removed auth-policy flag handling, and explicit `--auth-scheme` behavior.
- Transform-plan audit JSON coverage, including structured audit fields and the generated native direct HTTP invocation audit.
- Support matrix and diagnostic-code contract tests for `src/support.rs`, schema diagnostics, model unsupported-operation codes, `pp explain`, `pp check --json`, and `pp support --json` query behavior.
- Golden generated-output snapshot coverage for a fixed native direct HTTP fixture.
- Local corpus manifest validation for 25 local curated public API-shape fixtures, including `fixture_kind` provenance metadata, coverage tags, and expected diagnostic metadata.

## Standard

Run manually or by the weekly `Generated Workspace Smoke` workflow. When runtime templates change, run at least the MCP ignored smoke tests below (`mcp_errors` and `mcp_usability`) in addition to the fast profile.

```bash
cargo test --test petstore_smoke -- --ignored
cargo test --test slicing -- --ignored
cargo test --test openapi31 -- --ignored
cargo test --test bearer_smoke -- --ignored
cargo test --test apikey_smoke -- --ignored
cargo test --test basic_smoke -- --ignored
cargo test --test mcp_errors -- --ignored
cargo test --test mcp_usability -- --ignored
cargo test --test corpus -- --ignored
```

Purpose:

- Prove representative clean generated workspaces compile with `cargo build --release`.
- Keep full Petstore as strict native-subset rejection coverage and sliced Petstore as representative generated native workspace build coverage.
- Build and run a generated workspace from the OpenAPI 3.1 safe subset, including nullable `[T, null]`, `$defs`, JSON body fields, path params, repeated query-array serialization, and supported primitive schema annotation preservation.
- Exercise bearer, API key, and basic auth header behavior against local `mockito` servers.
- Exercise repeated query-array serialization as repeated query parameters.
- Exercise MCP error classification, `tools/list` pagination, and response shaping.
- Rebuild generated runtimes that expose CLI and MCP through generated native direct HTTP invocation metadata.
- Run local-only corpus `pp check --json` coverage across 25 curated public API-shape fixtures, write deterministic reports to `target/pp-corpus-coverage.json` and `target/pp-corpus-coverage.md` with diagnostic-code, coverage-tag, fixture-kind, and support-feature frequencies, and run generated build smoke for opted-in check-pass fixtures.

These are the first ignored tests promoted to scheduled/manual CI because they cover generated artifact correctness without external network dependencies.

## Deep

Run manually before release candidates or after large generator/runtime changes, especially changes to transform audits, auth selection, or generated CLI/MCP invocation.

```bash
cargo test --test dogfood -- --ignored
```

Optional large-spec manual checks should use strict, parser-ready OpenAPI 3.0 fixtures or specs that fit the documented OpenAPI 3.1 safe subset. Public specs that require shape repair, hidden compatibility behavior, fallback generation, or broad JSON Schema 2020-12 support are intentionally expected to fail until their source specs are fixed upstream or preprocessed outside `pp`.

Purpose:

- Exercise multiple fixture CLIs and MCP tool exposure.
- Recheck large-spec assumptions and strict-subset slicing before release.

## `pp validate`

`pp validate <workspace>` currently performs generated-workspace build validation:

```bash
pp validate ./out/ping
```

It intentionally has a narrow first contract: run `cargo build --release` in an emitted workspace and return the build result. Future validation levels can add runtime smoke checks without changing the meaning of the current build validation.
