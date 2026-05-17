# Support matrix

`pp` uses the crate-internal contract in `src/support.rs` as the stable support matrix ID, feature ID list, and emitted diagnostic-code inventory. This page is the human-readable explanation of that contract.

Matrix ID: `pp.strict-openapi-support.v1`

The same single source of truth is queryable from the CLI:

```bash
pp support --json
pp support --feature openapi.3_1.safe_subset --json
pp support --diagnostic direct_http.request_body_json_missing --json
```

Run `pp check <spec>` to evaluate a spec against this matrix without rendering a generated workspace. Human output shows diagnostic severity, strict behavior, remediation, and related support features. Check JSON includes `schema_version: "pp.check.v1"`, the matrix ID, facts/reports where available, diagnostics with additive explanation/support metadata, and unsupported operations with direct diagnostic codes.

## Supported / required input contract

- OpenAPI 3.0 strict subset: parsed and generated without source repair, fallback generation, or silent omission. Supported primitive schema annotations are preserved only when the schema already declares a supported primitive base type.
- OpenAPI 3.1 safe subset: primitive path/query parameters, exploded primitive query arrays, JSON request bodies, `components/schemas` and `$defs` references, nullable unions of the form `type: [T, null]`, and supported primitive schema annotations only when the base schema type is already supported.
- Required operation identity: every generated operation must declare an explicit stable `operationId`.
- Required runtime base URL: generated workspaces need an absolute `servers[0].url` or `--base-url` override.
- Parameters: primitive path/query parameters and exploded query arrays with primitive non-null items.
- Request bodies: `application/json` bodies, either flattened object fields or a single whole JSON `body` argument.
- Auth: no auth, HTTP bearer, header `apiKey`, and HTTP basic.

## Unsupported diagnostics

Supported primitive annotations are metadata preservation only; they do not infer missing `type`, widen accepted schema shapes, or make unsupported keywords valid. Unsupported features remain diagnostics; they are not compatibility rewrites, fallback paths, repairs, or hidden compatibility modes. Current unsupported classes include header/cookie parameters, non-form query style, non-simple path style, non-exploded query arrays, path arrays, non-primitive parameters, nullable required parameters, nullable query-array items, non-JSON or schemaless request bodies, unresolved schema references, unsupported `$ref` siblings, composition/conditionals, tuple arrays, `additionalProperties`, invalid schema types, and broad JSON Schema type unions.

User-facing unsupported messages are intentionally stable. Internally, unsupported paths also carry diagnostic codes from `src/support.rs`; `pp explain <diagnostic-code>` and `pp support --diagnostic <code> --json` expose the corresponding meaning, remediation, and related support features. The complete emitted inventory is `ALL_DIAGNOSTIC_CODES`, grouped under:

- `spec.*` for spec load/check diagnostics.
- `runtime.*` for runtime prerequisite diagnostics such as base URL selection.
- `model.*` for generator model-building diagnostics.
- `schema.*` for schema projection diagnostics.
- `direct_http.*` for native direct HTTP model-support diagnostics.
