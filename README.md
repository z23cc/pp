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

List operations in a spec for discovery and slicing:

```bash
pp inspect testdata/petstore.yaml --list-operations
```

Generate a CLI workspace from a strict OpenAPI 3.0 spec and build it:

```bash
cat > /tmp/ping.yaml <<'YAML'
openapi: 3.0.0
info:
  title: Ping API
  version: "1.0.0"
servers:
  - url: https://example.test
paths:
  /ping:
    get:
      operationId: ping
      responses:
        '200':
          description: ok
YAML
pp generate /tmp/ping.yaml -o ./out/ping --build
./out/ping/target/release/ping-api --help
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
pp generate stripe.yaml -o ./stripe --build
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

MCP tool calls use an audited direct HTTP adapter: generated operation metadata supplies the method, path template, path/query argument bindings, and JSON body bindings. Each generated workspace records the adapter in `pp-transform-plan.json` as `runtime.mcp_invocation.direct_http`. The human CLI path still uses the generated Progenitor CLI dispatch.

MCP tool calls return the full structured JSON response by default. Agent clients can opt into response shaping with reserved MCP-only parameters:

- `_pp_fields`: array of object dot paths to keep, for example `["name", "types", "stats"]`.
- `_pp_compact`: boolean that removes `null`, empty arrays, and empty objects from successful structured results.

These `_pp_` controls only apply to successful MCP tool results. CLI `--json` output and MCP error diagnostics are unchanged. OpenAPI parameters using the `_pp_` prefix are rejected during generation because that namespace is reserved by the wrapper.

## Spec preparation

`pp` keeps the parsed OpenAPI document strict: it does not rewrite, drop,
prune, or relax typed OpenAPI shapes to satisfy the Progenitor backend. Specs are
parsed, optionally sliced, inspected, modeled, and generated as-is; unsupported backend
shapes fail at model construction or backend code generation.

Generated workspaces also require an explicit base URL. `pp` uses `servers[0].url` from
the spec, or `--base-url <URL>` when the spec does not declare a server.

Generated commands and MCP tools require every selected operation to declare an explicit,
stable `operationId`. `pp inspect --list-operations` still shows discovery-only derived
IDs for unnamed operations, but generation fails until those operations are given an
`operationId` or excluded from the generated surface.

`pp inspect --reports` exposes structured preparation reports for explicit slicing.
Generated workspaces write `pp-transform-plan.json` with machine-readable audit entries
for runtime-generation seams. Audit entries may include structured fields such as
`action_kind`, `backend_requirement_id`, `before_json`, and `after_json`.

Current preparation behavior:

- Typed OpenAPI shapes are not rewritten, dropped, pruned, or relaxed for backend output.
- OpenAPI 3.1 input must be parseable by the current typed parser; `pp` no longer performs
  raw 3.1-to-3.0 repair passes.
- Operation slicing remains explicit and reports selected/dropped operations and pruned
  unreachable components.

## Auth

Generated CLIs currently support:

- no auth
- HTTP bearer via `<BIN>_TOKEN`
- header `apiKey` via `<BIN>_API_KEY`
- HTTP basic via `<BIN>_USER` and `<BIN>_PASSWORD`

By default, auth selection fails when multiple supported component security schemes are
selectable. Use `--auth-scheme <NAME>` to select a specific
`components.securitySchemes` entry; explicit scheme selection does not infer
query-parameter heuristics.

Example:

```bash
MY_API_TOKEN=foo ./out/my-api/target/release/my-api get-ping
```

## Known limitations

- OpenAPI 3.1 support is limited to shapes accepted by the current typed parser.
- OAuth2 flows are not implemented; use an explicit HTTP bearer scheme for token-based APIs.
- Very large specs can still expose upstream `progenitor` / `typify` codegen limits. `pp` currently carries a temporary typify fork patch for nullable-composition fixes; remove it only after upstream releases the fixes and the GitHub-scale regression still passes.

## Verification

Default tests stay fast:

```bash
cargo test
```

`pp validate <workspace>` runs `cargo build --release` in a generated workspace:

```bash
pp validate ./out/ping
```

Generated-workspace smoke tests are ignored in normal PR runs and covered by a manual/scheduled workflow. See `docs/verification.md` for the fast, standard, and deep verification profiles.

The standard smoke profile covers clean generated-workspace builds, expected Petstore backend rejection, auth headers, MCP error shapes, `tools/list` pagination, and MCP response shaping against local `mockito` servers.

## Contributing

Issues and PRs are welcome. Before submitting, run:

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
