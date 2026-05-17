# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Verification profiles are documented in `docs/verification.md`.
- Manual/scheduled generated-workspace smoke workflow for standard and deep profiles.
- OpenAPI 3.1 safe-subset generated-workspace smoke coverage.
- Internal support matrix and diagnostic-code contract for strict OpenAPI subset diagnostics.
- `pp explain` and `pp check` diagnostics now expose severity, strict behavior, remediation, and related support features.
- Local corpus expanded to 25 curated public API-shape fixtures with `fixture_kind` provenance metadata and deterministic coverage reports under `target/pp-corpus-coverage.{json,md}`.
- Corpus coverage reports now include diagnostic-code, coverage-tag, fixture-kind, and support-feature frequencies.

## [0.1.0] - TBD

### Added

- Initial release: OpenAPI YAML/JSON → installable Rust CLI workspace.
- `pp inspect`, `pp generate`, and build-only `pp validate` commands.
- Spec slicing by operation, tag, path prefix, and exclusion, with component pruning.
- Structured preparation/slicing reports with human-readable warning output.
- Auth support: none, bearer, header API key, and HTTP basic.
- Generated MCP stdio server with one tool per operation.
- MCP `tools/list` cursor pagination.
- MCP response shaping via `_pp_fields` and `_pp_compact`.
- Internal generation pipeline, API/MCP model layer, and backend adapter seam.
- Strict OpenAPI 3.1 safe-subset support for primitive path/query params, exploded primitive query arrays, JSON request bodies, `components/schemas` plus `$defs` refs, nullable `type: [T, null]` unions, and supported primitive annotations when the base schema type is already supported.

### Changed

- Wrapper rendering consumes precomputed model data instead of walking raw OpenAPI.
- Generated workspaces use the native direct HTTP runtime for both human CLI commands and MCP tools.
- Strict generation rejects unsupported selected operation shapes and unsupported OpenAPI 3.1 JSON Schema features instead of rewriting, falling back, or omitting them.

### Fixed

- Added generation-time checks for MCP tool names, reserved `_pp_` arguments, and generated CLI flag collisions.

