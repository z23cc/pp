# Preparation Report Inventory — 2026-05-16

This inventory reflects the current strict preparation path.

## Typed OpenAPI handling

No typed OpenAPI rewrite pass runs after parsing. The compiler preserves response variants, media types, request bodies, schema defaults, enum/property names, unsupported schema types, and response schemas for model/backend handling.

## Slicing reports

These reports are emitted by `src/spec/slice.rs` after explicit operation filtering and component pruning.

| Code | Group | Purpose |
| --- | --- | --- |
| `spec.slice.operations_filtered` | Slicing | Summarize kept/dropped operations. |
| `spec.slice.components_pruned` | Slicing | Summarize component map pruning. |

## Code ownership

- Report code constants and static inventory live in `src/spec/preparation_rules.rs`.
- Report structure lives in `src/spec/report.rs`.
- Parsing and optional slicing are coordinated by `src/spec/mod.rs`.
