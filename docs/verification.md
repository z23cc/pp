# Verification profiles

`pp` separates fast source checks from generated-artifact confidence checks so normal PRs stay quick while generated CLIs and MCP servers are still exercised regularly.

## Fast

Run on every push and pull request via `.github/workflows/ci.yml`.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Purpose:

- Rust formatting and lint health.
- Unit and integration tests that do not build generated release workspaces.
- Inspect/slicing/pipeline/model/backend behavior.
- Auth-selection policy coverage for legacy, fail-ambiguous, and explicit `--auth-scheme` behavior.
- Transform-plan audit JSON coverage, including structured audit fields and the runtime Progenitor CLI bridge audit.

## Standard

Run manually or by the weekly `Generated Workspace Smoke` workflow. When runtime templates change, run at least the MCP ignored smoke tests below (`mcp_errors` and `mcp_usability`) in addition to the fast profile.

```bash
cargo test --test petstore_smoke -- --ignored
cargo test --test slicing -- --ignored
cargo test --test bearer_smoke -- --ignored
cargo test --test apikey_smoke -- --ignored
cargo test --test basic_smoke -- --ignored
cargo test --test mcp_errors -- --ignored
cargo test --test mcp_usability -- --ignored
```

Purpose:

- Prove representative generated workspaces compile with `cargo build --release`.
- Preflight sliced generation with the petstore `store` tag so pruning remains covered outside fast PR CI.
- Exercise bearer, API key, and basic auth header behavior against local `mockito` servers.
- Exercise MCP error classification, `tools/list` pagination, and response shaping.
- Rebuild generated runtimes that currently expose MCP through the audited Progenitor CLI bridge.

These are the first ignored tests promoted to scheduled/manual CI because they cover generated artifact correctness without external network dependencies.

## Deep

Run manually before release candidates or after large generator/backend changes, especially changes to transform audits, auth selection, or generated MCP invocation.

```bash
cargo test --test dogfood -- --ignored
```

Optional large-spec manual checks should also include representative public specs documented under `docs/plans/`, especially GitHub REST slices/full generation when the fixture is available locally.

Purpose:

- Exercise multiple fixture CLIs and MCP tool exposure.
- Recheck large-spec assumptions and the temporary typify patch before release.

## `pp validate`

`pp validate <workspace>` currently performs generated-workspace build validation:

```bash
pp validate ./out/petstore
```

It intentionally has a narrow first contract: run `cargo build --release` in an emitted workspace and return the build result. Future validation levels can add runtime smoke checks without changing the meaning of the current build validation.
