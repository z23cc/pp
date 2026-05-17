# Release status

`pp` is preparing its first `0.1.0` release. The crate metadata is release-shaped, but publication should wait until the verification checklist below passes.

## Crates.io

- Package: `pp-cli`
- Binary: `pp`
- Current release target: `0.1.0`
- Publication status: not published yet

## Generation backend

The default generator is native direct HTTP. Generated human CLI commands and MCP tools share the same operation table and runtime request path. The emitted workspace contains a single generated crate and does not depend on an external OpenAPI client generator.

The supported input contract is a strict OpenAPI 3.0 subset plus the narrow OpenAPI 3.1 safe subset documented in [`docs/support-matrix.md`](support-matrix.md): explicit `operationId`, primitive path/query parameters, exploded primitive query arrays, JSON request bodies, `components/schemas` plus `$defs` references, nullable unions of the form `type: [T, null]`, supported primitive annotations only when the base schema type is already supported, and the documented auth schemes. Unsupported shapes fail during strict check/modeling or generated build validation; `pp` does not repair, mutate, fall back, silently omit, or apply hidden compatibility to specs/features to broaden support. The OpenAPI 3.1 support is not broad JSON Schema 2020-12 compliance.

## Release checklist

Before publishing `0.1.0`:

1. Run fast verification from `docs/verification.md`.
2. Run the standard generated-workspace smoke profile, including OpenAPI 3.1 safe-subset coverage, query-array runtime coverage, 25-fixture local corpus coverage report generation, and the generated sliced petstore smoke.
3. Run the deep fixture dogfood profile.
4. Re-run at least one large-spec or documented slice check when the fixture is available.
5. Confirm `CHANGELOG.md` has the intended `0.1.0` entry.
6. Confirm README badges and installation instructions match the publication state.
7. Tag the release only after generated artifact validation passes.
