# Support matrix

`pp` uses the crate-internal contract in `src/support.rs` as the stable support matrix ID, feature ID list, and emitted diagnostic-code inventory. This page is the human-readable explanation of that contract.

Matrix ID: `pp.strict-openapi-support.v1`

## Supported / required input contract

- OpenAPI 3.0 strict subset: parsed and generated without source repair, fallback generation, or silent omission.
- OpenAPI 3.1 safe subset: primitive path/query parameters, exploded primitive query arrays, JSON request bodies, `components/schemas` and `$defs` references, and nullable unions of the form `type: [T, null]`.
- Required operation identity: every generated operation must declare an explicit stable `operationId`.
- Required runtime base URL: generated workspaces need an absolute `servers[0].url` or `--base-url` override.
- Parameters: primitive path/query parameters and exploded query arrays with primitive non-null items.
- Request bodies: `application/json` bodies, either flattened object fields or a single whole JSON `body` argument.
- Auth: no auth, HTTP bearer, header `apiKey`, and HTTP basic.

## Unsupported diagnostics

Unsupported features remain diagnostics; they are not compatibility rewrites. Current unsupported classes include header/cookie parameters, non-form query style, non-simple path style, non-exploded query arrays, path arrays, non-primitive parameters, nullable required parameters, nullable query-array items, non-JSON or schemaless request bodies, unresolved schema references, `$ref` siblings, composition/conditionals, tuple arrays, `additionalProperties`, invalid schema types, and broad JSON Schema type unions.

User-facing unsupported messages are intentionally stable. Internally, unsupported paths also carry diagnostic codes from `src/support.rs`. The complete emitted inventory is `ALL_DIAGNOSTIC_CODES`, grouped under:

- `schema.*` for schema projection diagnostics.
- `direct_http.*` for native direct HTTP model-support diagnostics.

Generation requirements such as explicit `operationId` and absolute base URL are hard errors today; they are documented support requirements, not emitted diagnostic-code namespaces.
