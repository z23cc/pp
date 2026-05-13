# pp — Printing Press (Rust)

`pp` turns one OpenAPI 3.0 YAML/JSON spec into a buildable Rust CLI workspace.
It inspects the spec, links `progenitor` as a library for API client + command
generation, then renders the wrapper files that make the generated CLI runnable.

## Prerequisites

```bash
cargo build
```

## Quickstart

Inspect the facts `pp` derives from a spec:

```bash
cargo run -- inspect testdata/petstore.yaml
```

Generate a CLI workspace and build it:

```bash
cargo run -- generate testdata/petstore.yaml -o ./out/petstore --build
```

Run the generated binary help:

```bash
./out/petstore/target/release/swagger-petstore-open-api-3-0 --help
```

## Spec normalization

When an operation declares multiple request body media types, `pp` keeps
`application/json` if present, otherwise the first available media type. It
prints each normalization to stderr before generation. Progenitor 0.14 doesn't
yet support multi-media-type endpoints, so `pp` normalizes ahead of generation to
keep behavior predictable.

The same media-type rule applies to response content: `application/json` wins,
then the first available media type. This preserves the most useful CLI output
shape for typical APIs while avoiding progenitor's multi-content assertion.

When an operation declares multiple response variants, `pp` keeps one: `200`,
else the first 2xx code, else the first available entry such as `default`. This
trades away progenitor-side typed error handling for MVP compatibility; printed
CLIs still surface API errors through the generated error path.

Operations declaring a request body without a JSON Schema lose body input on the
CLI side; this matches progenitor's inability to generate a typed argument.

## Auth

For `http` bearer auth, the generated CLI reads `<BIN>_TOKEN`:

```bash
MY_API_TOKEN=foo ./out/my-api/target/release/my-api get-ping
```

For header `apiKey` auth, it reads `<BIN>_API_KEY` and sends the configured
header name, for example `X-API-Key`.

For `http` basic auth, it reads `<BIN>_USER` and `<BIN>_PASSWORD` and sends an
`Authorization: Basic ...` header when both are set.

## Tests

Default tests stay fast:

```bash
cargo test
```

Smoke tests generate real CLIs and run `cargo build --release`, so they are
ignored by default. Run them explicitly when needed:

```bash
cargo test -- --ignored
```

The smoke suite covers petstore generation/build plus parameterized dispatcher
commands and bearer/API-key headers against a local `mockito` server.
