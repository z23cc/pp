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
- Spec slicing by operation, tag, path prefix, and exclusion, with component pruning.
- Structured preparation/slicing reports with human-readable warning output.
- Auth support: none, bearer, header API key, and HTTP basic.
- Generated MCP stdio server with one tool per operation.
- MCP `tools/list` cursor pagination.
- MCP response shaping via `_pp_fields` and `_pp_compact`.
- Internal generation pipeline, API/MCP model layer, and backend adapter seam.

### Changed

- Wrapper rendering consumes precomputed model data instead of walking raw OpenAPI.
- Generated workspaces use the native direct HTTP runtime for both human CLI commands and MCP tools.
- Strict generation rejects unsupported selected operation shapes instead of rewriting or omitting them.

### Fixed

- Added generation-time checks for MCP tool names, reserved `_pp_` arguments, and generated CLI flag collisions.

