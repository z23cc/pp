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
./out/petstore/target/release/swagger-petstore-openapi-30 --help
```

## Auth

For `http` bearer auth, the generated CLI reads `<BIN>_TOKEN`:

```bash
MY_API_TOKEN=foo ./out/my-api/target/release/my-api get-ping
```

For header `apiKey` auth, it reads `<BIN>_API_KEY` and sends the configured
header name, for example `X-API-Key`.

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
