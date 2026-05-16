# pp — OpenAPI to installable Rust CLIs

[![CI](https://github.com/z23cc/pp/actions/workflows/ci.yml/badge.svg)](https://github.com/z23cc/pp/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![release status](https://img.shields.io/badge/release-0.1.0%20candidate-blue.svg)](docs/release-status.md)

`pp` turns one OpenAPI YAML/JSON spec into a buildable Rust CLI workspace. It
uses `progenitor` as an in-process codegen library for API client + command
generation, then renders wrapper files that make the generated CLI runnable.

The Cargo package is `pp-cli`; the installed binary is `pp`. The project is preparing a `0.1.0` crates.io release; see `docs/release-status.md` for the current publication checklist and temporary dependency notes.

## Quickstart

```bash
git clone https://github.com/z23cc/pp.git
cd pp
cargo build --release
cargo install --path .
pp --help
```

Inspect the facts `pp` derives from a spec:

```bash
pp inspect testdata/petstore.yaml --allow-compat-normalization
```

Generate a CLI workspace and build it:

```bash
pp generate testdata/petstore.yaml -o ./out/petstore --allow-compat-normalization --build
./out/petstore/target/release/swagger-petstore --help
```

## What pp is good for

| User type | Use pp for |
| --- | --- |
| Scripters | Install one typed binary from an OpenAPI spec and call endpoints from shell scripts. |
| Agents | Expose the same generated binary as an MCP stdio server with one tool per operation. |
| DevOps | Generate reproducible Rust CLIs that share auth, base URL, and request handling. |

## Agent users

Every generated binary supports human CLI commands and an MCP stdio server:

```bash
pp generate stripe.yaml -o ./stripe --allow-compat-normalization --build
cargo install --path ./stripe
stripe charges_retrieve --id ch_123
stripe mcp
```

Claude Desktop config:

```json
{
  "mcpServers": {
    "stripe": {
      "command": "stripe",
      "args": ["mcp"],
      "env": {
        "STRIPE_TOKEN": "sk_test_..."
      }
    }
  }
}
```

Example `tools/list` entry:

```json
{
  "name": "find_pets_by_status",
  "description": "Finds Pets by status. [auth: SWAGGER_PETSTORE_API_KEY env var]",
  "inputSchema": {
    "type": "object",
    "properties": { "status": { "type": "string" } },
    "required": ["status"]
  }
}
```

Use `--json` with normal CLI commands to get one structured JSON value on stdout.
Human-readable progress and diagnostics stay on stderr.

MCP `tools/list` uses standard cursor pagination with a server-defined page size.
Clients should follow `nextCursor` until it is absent to discover every generated tool.

MCP tool calls currently use an audited Progenitor CLI bridge adapter: MCP JSON arguments are adapted into generated CLI argv/Clap dispatch before reaching the generated operation executor. This keeps CLI and MCP behavior aligned today, and each generated workspace records the adapter in `pp-transform-plan.json` as `runtime.mcp_invocation.progenitor_cli_bridge`. Direct typed invocation is blocked until generated Progenitor output exposes stable operation metadata such as method names, parameter setters, request body types, and response types.

MCP tool calls return the full structured JSON response by default. Agent clients can opt into response shaping with reserved MCP-only parameters:

- `_pp_fields`: array of object dot paths to keep, for example `["name", "types", "stats"]`.
- `_pp_compact`: boolean that removes `null`, empty arrays, and empty objects from successful structured results.

These `_pp_` controls only apply to successful MCP tool results. CLI `--json` output and MCP error diagnostics are unchanged. OpenAPI parameters using the `_pp_` prefix are rejected during generation because that namespace is reserved by the wrapper.

## Spec normalization

`pp` is strict by default: compatibility rewrites, lossy drops, backend workarounds,
and unsafe fallback replacements fail generation instead of silently proceeding. Pass
`--allow-compat-normalization` to explicitly permit these transformations for real-world
specs that require progenitor compatibility.

Generated workspaces also require an explicit base URL. `pp` uses `servers[0].url` from
the spec, or `--base-url <URL>` when the spec does not declare a server; it no longer
falls back to `http://localhost`.

Generated commands and MCP tools require every selected operation to declare an explicit,
stable `operationId`. `pp inspect --list-operations` still shows discovery-only derived
IDs for unnamed operations, but generation fails until those operations are given an
`operationId` or excluded from the generated surface.

When compatibility normalization is allowed, `pp` prints each normalization to stderr,
exposes structured report entries through `pp inspect --reports`, and writes
`pp-transform-plan.json` into generated workspaces. Prefer targeted approval with
`--allow-effect <effect>` or `--allow-report-code <code>` when only one transform class
or report code is acceptable.

The transform plan also includes machine-readable audit entries for applied raw repairs,
typed normalization, backend source transforms, and runtime-generation seams. Audit entries
keep their existing text fields and may include structured fields such as `target_pointer`,
`action_kind`, `backend_requirement_id`, `before_json`, and `after_json`.

Available transform effects are `lossless_repair`, `explicit_selection`, `lossy_rewrite`,
`semantic_drop`, `backend_workaround`, and `unsafe_fallback`. The `unsafe_fallback`
effect is explicit-approval vocabulary for last-resort compatibility replacement; it is
not an implicit fallback path in strict mode.

Current compatibility rules:

- Request body media types: keep `application/json` when present, otherwise fail unless
  compatibility normalization is explicitly allowed.
- Response media types: keep `application/json` when present, otherwise fail unless
  compatibility normalization is explicitly allowed.
- Response variants: keep `200`, else the first 2xx response, else the first
  available response such as `default`; strict mode rejects this pruning by default.
- Schemaless request bodies: drop CLI body input when no JSON Schema is present.
- OpenAPI 3.1: downgrade supported 3.1 shapes into the 3.0 parser path only when
  compatibility normalization is explicitly allowed.
- Enum collisions, property name collisions, and unsupported schema types are
  rewritten when compatibility normalization is explicitly allowed so codegen can continue.

## Auth

Generated CLIs currently support:

- no auth
- HTTP bearer via `<BIN>_TOKEN`
- header `apiKey` via `<BIN>_API_KEY`
- HTTP basic via `<BIN>_USER` and `<BIN>_PASSWORD`
- OAuth2 treated as bearer token input

By default, auth selection remains compatibility-preserving: when multiple supported component
security schemes are present, the `legacy` policy keeps the first supported scheme in component order.
Use `--auth-policy fail-ambiguous` with `inspect` or `generate` to fail instead when more
than one supported component scheme is selectable. Use `--auth-scheme <NAME>` to select a
specific `components.securitySchemes` entry; this overrides `--auth-policy` and does not
fall back to query-parameter heuristics.

Example:

```bash
MY_API_TOKEN=foo ./out/my-api/target/release/my-api get-ping
```

## Known limitations

- OpenAPI 3.1 support is a downgrade pass, not a full native 3.1 implementation.
- OAuth2 is modeled as bearer-token input only.
- Very large specs can still expose upstream `progenitor` / `typify` codegen limits. `pp` currently carries a temporary typify fork patch for nullable-composition fixes; remove it only after upstream releases the fixes and the GitHub-scale regression still passes.
- The Progenitor backend still carries named generated-source transforms: CLI parser-compatibility transforms for generated Progenitor surfaces, plus a local unexpected-response diagnostic transform for readable error bodies. These are audited backend-adapter source transforms, not hidden local normalization.

## Verification

Default tests stay fast:

```bash
cargo test
```

`pp validate <workspace>` runs `cargo build --release` in a generated workspace:

```bash
pp validate ./out/petstore
```

Generated-workspace smoke tests are ignored in normal PR runs and covered by a manual/scheduled workflow. See `docs/verification.md` for the fast, standard, and deep verification profiles.

The standard smoke profile covers petstore generation/build, auth headers, MCP error shapes, `tools/list` pagination, and MCP response shaping against local `mockito` servers.

## Contributing

Issues and PRs are welcome. Before submitting, run:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```
