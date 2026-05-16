# Normalization Rule Inventory — 2026-05-16

This inventory maps current compatibility transforms to structured report codes. It is intentionally descriptive rather than configurable; the goal is to make future rule extraction auditable while preserving current behavior.

## Pre-parse tolerance

These rules run before typed `openapiv3::OpenAPI` deserialization in `src/spec/pre_parse.rs`.

| Code | Group | Purpose |
| --- | --- | --- |
| `spec.pre_parse.openapi_31_downgraded` | OpenAPI downgrade | Convert supported OpenAPI 3.1 syntax into the 3.0 parser path. |
| `spec.pre_parse.numeric_bounds_clamped` | Pre-parse tolerance | Clamp integer bounds that exceed parser-supported i64 limits. |
| `spec.pre_parse.tag_descriptions_replaced` | Pre-parse tolerance | Replace non-string top-level tag descriptions so vendor metadata can parse. |
| `spec.pre_parse.ref_only_operations_replaced` | Pre-parse tolerance | Replace ref-only operations with parseable placeholder operations. |

## Typed normalization

These rules run after typed parsing in `src/spec/normalize.rs`.

| Code | Group | Purpose |
| --- | --- | --- |
| `spec.normalize.operation_ids_shortened` | Operation naming | Shorten verbose operation IDs while preserving uniqueness. |
| `spec.normalize.response_variants_pruned` | Progenitor compatibility | Keep one response variant per operation. |
| `spec.normalize.content_types_pruned` | Progenitor compatibility | Keep one supported request/response content type. |
| `spec.normalize.schemaless_request_body_dropped` | Progenitor compatibility | Drop body CLI input when request body content has no schema. |
| `spec.normalize.unsupported_request_bodies_dropped` | Progenitor compatibility | Drop operations with no progenitor-supported request body media type. |
| `spec.normalize.deep_object_query_params_rewritten` | Progenitor compatibility | Rewrite unsupported `deepObject` query style to `form`. |
| `spec.normalize.optional_object_query_params_dropped` | Progenitor compatibility | Drop optional object-shaped query params that break generated builders. |
| `spec.normalize.schema_defaults_dropped` | Progenitor compatibility | Drop schema defaults that typify/progenitor may reject. |
| `spec.normalize.enum_constraint_dropped` | Progenitor compatibility | Drop enum constraints that collide after Rust identifier sanitization. |
| `spec.normalize.unsupported_schema_type_replaced` | Progenitor compatibility | Replace unsupported schema type names with fallback schemas. |
| `spec.normalize.properties_colliding_dropped` | Progenitor compatibility | Drop object properties that collide after Rust field-name sanitization. |
| `spec.normalize.response_schemas_relaxed` | Response relaxation | Relax response-only schemas for tolerant deserialization. |

## Slicing reports

These reports are emitted by `src/spec/slice.rs` after operation filtering and component pruning.

| Code | Group | Purpose |
| --- | --- | --- |
| `spec.slice.operations_filtered` | Slicing | Summarize kept/dropped operations. |
| `spec.slice.components_pruned` | Slicing | Summarize component map pruning. |

## Code ownership

- Code constants and static inventory live in `src/spec/normalization_rules.rs`.
- String-level pre-parse rules live in `src/spec/pre_parse.rs`.
- Typed progenitor-compatibility and response-relaxation rules still live in `src/spec/normalize.rs`, but now enter through explicit group functions.
