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
- Inspect/slicing/pipeline/model/backend behavior.
- Auth-selection coverage for fail-ambiguous defaults, removed auth-policy flag handling, and explicit `--auth-scheme` behavior.
- Transform-plan audit JSON coverage, including structured audit fields and the generated native direct HTTP invocation audit.
- Support matrix and diagnostic-code contract tests for `src/support.rs`, schema diagnostics, and model unsupported-operation codes.

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
```

Purpose:

- Prove representative clean generated workspaces compile with `cargo build --release`.
- Keep full Petstore as strict native-subset rejection coverage and sliced Petstore as representative generated native workspace build coverage.
- Build and run a generated workspace from the OpenAPI 3.1 safe subset, including nullable `[T, null]`, `$defs`, JSON body fields, path params, and repeated query-array serialization.
- Exercise bearer, API key, and basic auth header behavior against local `mockito` servers.
- Exercise repeated query-array serialization as repeated query parameters.
- Exercise MCP error classification, `tools/list` pagination, and response shaping.
- Rebuild generated runtimes that expose CLI and MCP through generated native direct HTTP invocation metadata.

These are the first ignored tests promoted to scheduled/manual CI because they cover generated artifact correctness without external network dependencies.

## Deep

Run manually before release candidates or after large generator/runtime changes, especially changes to transform audits, auth selection, or generated CLI/MCP invocation.

```bash
cargo test --test dogfood -- --ignored
```

Optional large-spec manual checks should use strict, parser-ready OpenAPI 3.0 fixtures or specs that fit the documented OpenAPI 3.1 safe subset. Public specs that require shape repair or broad JSON Schema 2020-12 support are intentionally expected to fail until their source specs are fixed upstream or preprocessed outside `pp`.

Purpose:

- Exercise multiple fixture CLIs and MCP tool exposure.
- Recheck large-spec assumptions and strict-subset slicing before release.

## `pp validate`

`pp validate <workspace>` currently performs generated-workspace build validation:

```bash
pp validate ./out/ping
```

It intentionally has a narrow first contract: run `cargo build --release` in an emitted workspace and return the build result. Future validation levels can add runtime smoke checks without changing the meaning of the current build validation.
