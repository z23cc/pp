# pp — OpenAPI to installable Rust CLIs

[![CI](https://github.com/z23cc/pp/actions/workflows/ci.yml/badge.svg)](https://github.com/z23cc/pp/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![crates.io](https://img.shields.io/badge/crates.io-TODO-lightgrey.svg)](https://crates.io/crates/pp-cli)

`pp` turns one OpenAPI YAML/JSON spec into a buildable Rust CLI workspace. It
uses `progenitor` as an in-process codegen library for API client + command
generation, then renders wrapper files that make the generated CLI runnable.

The Cargo package is `pp-cli`; the installed binary is `pp`.

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
pp inspect testdata/petstore.yaml
```

Generate a CLI workspace and build it:

```bash
pp generate testdata/petstore.yaml -o ./out/petstore --build
./out/petstore/target/release/swagger-petstore-open-api-3-0 --help
```

## Spec normalization

`pp` normalizes specs before handing them to `progenitor`. It prints each
normalization to stderr so generation stays transparent.

Current rules:

- Request body media types: keep `application/json` when present, otherwise the
  first available media type.
- Response media types: keep `application/json` when present, otherwise the
  first available media type.
- Response variants: keep `200`, else the first 2xx response, else the first
  available response such as `default`.
- Schemaless request bodies: drop CLI body input when no JSON Schema is present.
- OpenAPI 3.1: downgrade supported 3.1 shapes into the 3.0 parser path.
- Enum collisions, property name collisions, and unsupported schema types are
  rewritten when possible so codegen can continue.

## Auth

Generated CLIs currently support:

- no auth
- HTTP bearer via `<BIN>_TOKEN`
- header `apiKey` via `<BIN>_API_KEY`
- HTTP basic via `<BIN>_USER` and `<BIN>_PASSWORD`
- OAuth2 treated as bearer token input

Example:

```bash
MY_API_TOKEN=foo ./out/my-api/target/release/my-api get-ping
```

## Known limitations

- OpenAPI 3.1 support is a downgrade pass, not a full native 3.1 implementation.
- OAuth2 is modeled as bearer-token input only.
- Very large specs can expose upstream `progenitor` / `typify` codegen limits;
  GitHub-scale specs are currently affected by
  [oxidecomputer/typify#1011](https://github.com/oxidecomputer/typify/issues/1011).

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

## Contributing

Issues and PRs are welcome. Before submitting, run:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
