# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Verification profiles are documented in `docs/verification.md`.
- Manual/scheduled generated-workspace smoke workflow for standard and deep profiles.

## [0.1.0] - TBD

### Added

- Initial release: OpenAPI YAML/JSON → installable Rust CLI workspace.
- `pp inspect`, `pp generate`, and build-only `pp validate` commands.
- OpenAPI 3.1 downgrade/tolerance path for supported 3.1 shapes.
- Spec slicing by operation, tag, path prefix, and exclusion, with component pruning.
- Structured normalization/slicing reports with human-readable warning output.
- Auth support: none, bearer, header API key, HTTP basic, and OAuth2-as-bearer.
- Generated MCP stdio server with one tool per operation.
- MCP `tools/list` cursor pagination.
- MCP response shaping via `_pp_fields` and `_pp_compact`.
- Internal generation pipeline, API/MCP model layer, backend adapter seam, and named generated-source transforms.

### Changed

- Wrapper rendering consumes precomputed model data instead of walking raw OpenAPI.
- Progenitor-generated source patches are named transforms with focused tests.
- Large-spec support currently relies on a temporary typify fork patch documented in `docs/release-status.md`.

### Fixed

- Preserved upstream error response bodies in generated unexpected-response diagnostics.
- Hardened MCP temporary JSON body filenames against repeated-call reuse in one server process.
- Added generation-time checks for MCP tool names, reserved `_pp_` arguments, and generated CLI flag collisions.

