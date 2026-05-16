# Long-term Architecture Implementation Plan — 2026-05-16

## Goal

Turn the long-term architecture direction in `docs/plans/long-term-architecture-2026-05-16.md` into an executable implementation sequence. The work should preserve current `pp` behavior while creating deeper seams for pipeline orchestration, structured normalization reports, API/MCP modeling, backend isolation, and generated-artifact verification.

## Background

- Current generation is orchestrated directly from the CLI. `src/main.rs:9-11` parses `Cli` and calls `Cli::run()`. `src/cli/mod.rs:123-193` loads the spec, prints warnings, derives names, builds `WrapperManifest`, calls `progenitor_driver::generate`, calls `render::render`, and optionally runs `cargo build --release`.
- CLI slice flags are converted to `spec::LoadOptions` in `src/cli/mod.rs:72-84`, then `load()` / `load_with_options()` in `src/spec/mod.rs:52-91` handle file read, parse, normalize, optional slicing, and fact derivation.
- The long-term plan names the target compiler stages: `RawSpec`, `TolerantSpec`, `ParsedSpec`, `NormalizedSpec`, `SlicedSpec`, `ApiModel`, `WorkspacePlan`, `GeneratedWorkspace`, and `VerifiedArtifact` (`docs/plans/long-term-architecture-2026-05-16.md:23-35`). This implementation plan moves toward those stages without requiring a rewrite.
- `validate` exists but is not implemented; `src/cli/mod.rs:194-200` currently bails with a placeholder. Current build validation is embedded in `generate --build` at `src/cli/mod.rs:178-192`.
- Pre-deserialization tolerance is inside `parse()` / `pre_normalize_yaml()` (`src/spec/mod.rs:147-202`). It currently downgrades OpenAPI 3.1, clamps numeric bounds, normalizes top-level tag descriptions, and replaces ref-only operations before typed deserialization.
- Typed normalization is concentrated in `normalize::normalize()` (`src/spec/normalize.rs:19`) and manually walks components and path operation slots (`src/spec/normalize.rs:36-108`). It accumulates user-facing warning strings through `NormalizeStats` and direct warning pushes.
- Slicing is already a useful seam. `src/spec/slice.rs:9-50` defines `SliceOptions`, `OperationListing`, `SliceReport`, and `PrunedComponents`; `slice_openapi()` filters operations and prunes reachable components (`src/spec/slice.rs:82-181`).
- Operation traversal is duplicated across `count_operations()` (`src/spec/mod.rs:509`), `derive_query_api_key_auth()` (`src/spec/mod.rs:578`), `normalize::normalize()` (`src/spec/normalize.rs:36-108`), render MCP generation (`src/render/mod.rs:173-252`), and slicing (`src/spec/slice.rs:92-163`). Schema/reference graph traversal is duplicated between normalization response relaxation and slicing component pruning.
- Rendering currently mixes templating with OpenAPI semantic modeling. `WrapperManifest` is defined in `src/render/mod.rs:27-41`, `with_openapi()` populates MCP tools from raw OpenAPI (`src/render/mod.rs:89-92`), and `render()` writes templates (`src/render/mod.rs:96-148`).
- MCP tool modeling currently lives in render. `McpTool` and `McpArg` are in `src/render/mod.rs:43-60`; `mcp_tools()` and `push_operation()` walk paths and operations (`src/render/mod.rs:173-323`); `add_parameter()` and `add_body()` lower params/body fields into MCP args (`src/render/mod.rs:353-474`).
- Known MCP/modeling risks include `operationId.to_snake_case()` collisions (`src/render/mod.rs:269`), flattened body fields overwriting parameter schema names (`src/render/mod.rs:430-445`), synthetic `body` collisions (`src/render/mod.rs:456-473`), and temp body filenames using bin/tool/pid only in `src/render/templates/mcp.rs.j2:239-249`.
- Runtime wrapper seams already exist in templates: generated entrypoint `src/render/templates/main.rs.j2:8-11`, command dispatch in `src/render/templates/cli_builder.rs.j2:4-70`, auth in `src/render/templates/auth.rs.j2:6-30`, runtime context in `src/render/templates/context.rs.j2:5-41`, output hooks in `src/render/templates/print.rs.j2:77-166`, and MCP error/result shaping in `src/render/templates/mcp.rs.j2:102-207`.
- Progenitor is currently a concrete integration, not an adapter. `progenitor_driver::generate()` configures `GenerationSettings`, runs progenitor token generation, formats with `prettyplease`, applies string/regex source transforms, and writes `api/src/lib.rs` (`src/progenitor_driver/mod.rs:12-64`). Fragile transforms live at `src/progenitor_driver/mod.rs:31-58` and `src/progenitor_driver/mod.rs:67-82`.
- Parser replacement has already been investigated and rejected for this bug class. `oas3` failed on the same OpenAI numeric-bound and DigitalOcean tag-description cases, so the chosen direction is tolerant pre-normalization before typed deserialization, not parser swap (`docs/probe-oas3-2026-05-13.md:14-23`, `docs/probe-oas3-2026-05-13.md:41-45`).
- Oxide prior art supports treating progenitor output as a substrate plus wrapper overlays, and normalizing/curating specs before progenitor (`docs/probe-oxide-patterns-2026-05-13.md:52-60`).
- Existing CI runs fmt, clippy, and `cargo test --all` only (`.github/workflows/ci.yml:15-18`). Ignored smoke tests validate generated workspace builds, auth headers, MCP errors/usability, and fixture dogfood, but they are not run in CI (`tests/common/mod.rs:8-16`, `tests/mcp_usability.rs:49-72`, `tests/mcp_errors.rs:7-110`).
- Prior docs record GitHub-scale generation/build success with the typify fork, generated Clap parser patches, MCP schema string parsing, and operation slicing (`docs/plans/typify-patch-and-slicing-2026-05-16.md:5-28`, `docs/plans/typify-patch-and-slicing-2026-05-16.md:126-148`).
- Release metadata still shows pre-release gaps: README crates.io badge is TODO (`README.md:3-5`), changelog is only `[Unreleased]` (`CHANGELOG.md:7-16`), and the typify fork remains a temporary dependency strategy.

## Approach

Use a behavior-preserving sequence rather than a broad rewrite. The first milestone makes existing generated artifacts safer to change: implement `validate`, add targeted slicing/validation coverage, and centralize build verification. The second milestone extracts a pipeline request/result seam so the CLI stops owning generation orchestration. Later milestones introduce structured reports, API/MCP modeling, rendering cleanup, backend isolation, and verification profiles.

Design choices for this plan:

- The pipeline is **internal only** for now. It should not be treated as a stable public library API.
- Structured reports are **internal first**. Existing stderr warning text remains the compatibility contract; machine-readable `inspect` report output is future work.
- The first model layer is **MCP-focused and narrow**. It consumes the already-normalized/sliced `openapiv3::OpenAPI` and produces wrapper/MCP model data. It does not replace the OpenAPI representation yet.
- `WrapperManifest` should remain as the render-facing template manifest initially, but `WrapperManifest::with_openapi()` should be replaced by model construction once the model exists.
- Progenitor remains a one-way backend adapter for now. It should not feed naming or constraint hints back into the model in this phase.
- Large-spec verification should be manual or scheduled first; PR CI should stay fast until generated-workspace smoke runtime is known.

## Execution Status

- [x] Work item 1 — build-only `validate`: implemented in `src/cli/mod.rs` with focused coverage in `tests/validate.rs`.
- [x] Work item 2 — slicing behavior coverage: implemented for inspect/list/slice filters in `tests/slicing.rs`, with component-pruning coverage extended in `src/spec/slice.rs`.
- [x] Work item 3 — internal pipeline request/result seam: implemented in `src/pipeline/mod.rs`; `src/cli/mod.rs` now maps args to pipeline requests and prints progress.
- [x] Work item 4 — structured report entries internally: implemented via `src/spec/report.rs`, with reports carried through spec loading and pipeline while preserving formatted warning output.
- [x] Work item 5 — normalization rule inventory and first rule-group split: implemented via `src/spec/normalization_rules.rs`, `src/spec/pre_parse.rs`, grouped typed-normalization entrypoints in `src/spec/normalize.rs`, and documented in `docs/plans/normalization-rule-inventory-2026-05-16.md`.
- [x] Work item 6 — minimal API/MCP model layer: implemented in `src/model/mod.rs`; MCP tool construction now lives in the model layer and is invoked from the pipeline before render.
- [x] Work item 7 — model-level MCP collision checks and runtime temp-file hardening: implemented in `src/model/mod.rs` and `src/render/templates/mcp.rs.j2`, with focused tests.
- [x] Work item 8 — thin rendering to workspace emission: `ApiModel` construction now happens in `src/pipeline/mod.rs`; `src/render/mod.rs` only consumes precomputed `McpTool` data in `WrapperManifest`.
- [x] Work item 9 — progenitor backend adapter seam: implemented in `src/backend/mod.rs`; the pipeline now invokes `ProgenitorBackend` through the `ApiBackend` trait instead of calling `progenitor_driver` directly.
- [x] Work item 10 — named generated-source transforms: `src/progenitor_driver/mod.rs` now applies named source transforms with focused tests for string-vector parsers, complex Clap parsers, and unexpected-response body preservation.
- [x] Work item 11 — verification profiles and CI shape: documented in `docs/verification.md`, with a manual/scheduled generated-workspace smoke workflow at `.github/workflows/generated-smoke.yml`.
- [x] Work item 12 — release metadata and dependency-status cleanup: README, CHANGELOG, Cargo metadata, and `docs/release-status.md` now describe the 0.1.0 candidate, verification checklist, and temporary typify patch removal condition.

Latest focused verification: `cargo test --all` (61 passed, 9 ignored), `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` passed on 2026-05-16 after Work Items 11–12; `pp generate testdata/petstore.yaml` also passed after Work Item 10.

## Work Items

### 1. Implement build-only `validate`

**Goal:** Make `pp validate <workspace>` real and share build validation with `generate --build`.

**Done when:**

- `pp validate <workspace>` runs `cargo build --release` in the supplied generated workspace.
- `generate --build` and `validate` use the same validation helper.
- Existing `generate --build` stderr/error behavior is preserved unless tests approve a deliberate improvement.
- Validate has success and failure tests owned by this work item.

**Key files:** `src/cli/mod.rs:178-200`, `tests/common/mod.rs:8-16`, new or updated validation tests.

**Dependencies:** None.

**Size:** S.

### 2. Add behavior-locking coverage for slicing and smoke classification

**Goal:** Protect the current behavior before extracting orchestration.

**Done when:**

- Tests cover `--include-operation`, `--include-tag`, `--include-path-prefix`, `--exclude-operation`, and `inspect --list-operations`.
- Tests cover empty-slice behavior and component pruning at either unit or integration level.
- The test suite documents which generated-workspace tests are fast PR checks and which remain ignored/manual.
- Validate tests are not duplicated here; they belong to work item 1.

**Key files:** `tests/common/mod.rs`, `tests/*_smoke.rs`, `tests/mcp_usability.rs`, `src/spec/slice.rs:718-794`, `.github/workflows/ci.yml:15-18` if adding a manual/scheduled job.

**Dependencies:** Work item 1.

**Size:** M.

### 3. Extract an internal pipeline request/result seam

**Goal:** Move generation orchestration out of the CLI without changing generated output.

**Done when:**

- A new internal `pipeline` module owns the generate flow currently in `src/cli/mod.rs:123-193`.
- The entry point is a simple generate function that accepts a request and returns a result. The request should include spec path, output path, optional binary name, build/validate flag, and load options. The result should include derived facts, formatted warnings, output path, and optional validation result.
- The CLI maps clap arguments into the request and formats the result; it no longer directly calls each generation stage.
- `inspect`, `inspect --list-operations`, and sliced inspect keep their current stdout/stderr behavior. If helper functions are shared with the pipeline, they remain internal.
- Existing generate, inspect, and smoke behavior remains stable.

**Key files:** `src/cli/mod.rs`, `src/main.rs`, new `src/pipeline/mod.rs`, `src/spec/mod.rs`, `src/render/mod.rs`, `src/progenitor_driver/mod.rs`.

**Dependencies:** Work items 1–2.

**Size:** M.

### 4. Introduce structured report entries internally

**Goal:** Replace raw warning accumulation with typed report data while preserving current user-facing warning text.

**Done when:**

- A `src/spec/report.rs` module owns spec-stage report types and warning formatting.
- Report entries have a stage enum, severity enum, stable string code, message, and optional subject such as operation/schema/component.
- Current CLI warning strings are produced by formatting structured entries at the boundary.
- `LoadedSpec` or the pipeline result can carry both formatted warnings and structured reports during the transition.
- Unit tests assert at least one pre-parse tolerance report, one typed normalization report, and one slicing report.

**Key files:** `src/spec/mod.rs`, `src/spec/normalize.rs`, `src/spec/slice.rs`, new `src/spec/report.rs`, tests in `src/spec/*`.

**Dependencies:** Work item 3 for pipeline surfacing. The report work is structurally inside `src/spec`, but sequencing it after the pipeline avoids changing diagnostics before the orchestration seam exists.

**Size:** M.

### 5. Inventory and split normalization into rule groups

**Goal:** Make normalization auditable and testable without locking in an awkward module split too early.

**Done when:**

- The first commit in this item inventories existing normalization rules and maps them to report codes.
- Pre-deserialization tolerance and typed normalization are separated enough that reports can identify the stage that changed the spec.
- Natural rule groups are extracted from the inventory. Expected starting groups include pre-parse tolerance, OpenAPI version downgrade, progenitor compatibility, response relaxation, and operation naming, but the implementer may adjust the grouping if the inventory shows a better split.
- Each group emits structured report entries through the mechanism from work item 4.
- Existing normalization tests continue to pass with minimal fixture churn.
- The top-level normalization interface remains simple for the pipeline.

**Key files:** `src/spec/mod.rs:147-202`, `src/spec/normalize.rs:19-160`, new files under `src/spec/normalize/` or equivalent, existing normalize tests in `src/spec/normalize.rs:1221-1839`.

**Dependencies:** Work item 4.

**Size:** L.

### 6. Add a minimal API/MCP model layer

**Goal:** Move MCP tool construction decisions out of render while avoiding a full OpenAPI replacement model.

**Done when:**

- A new model layer consumes a normalized and sliced `openapiv3::OpenAPI` and produces current `McpTool` / `McpArg` equivalent data.
- Operation naming logic, including `operationId.to_snake_case()`, moves from render into the model layer.
- `WrapperManifest::with_openapi()` is removed or reduced to a compatibility wrapper that delegates to the model.
- Render consumes model data for MCP tools instead of walking raw OpenAPI.
- Existing MCP usability and error tests continue to pass.
- The model starts narrow: operation names, descriptions, params, body fields, input schema, auth hint, and wrapper-reserved args. It does not attempt to model every OpenAPI schema feature yet.

**Key files:** new `src/model/mod.rs`, `src/render/mod.rs:43-60`, `src/render/mod.rs:173-535`, `src/render/templates/mcp.rs.j2`, `tests/mcp_usability.rs`, `tests/mcp_errors.rs`.

**Dependencies:** Work items 3–4. Work item 5 is not a hard dependency; the model can consume the same normalized `openapiv3::OpenAPI` before normalization is fully split.

**Size:** L.

### 7. Add model-level MCP collision checks and a separate runtime temp-file hardening patch

**Goal:** Centralize MCP naming/body flattening safety, while treating temp-file uniqueness as runtime hardening rather than model design.

**Done when:**

- The model detects and reports collisions for the MCP naming/body-flattening risks identified in Background.
- `_pp_` reserved namespace checks remain enforced and are covered for params and body fields.
- Collision errors point to the affected operation/tool and argument where available.
- The MCP temp JSON body filename no longer relies only on bin/tool/pid, or a separate issue/plan item is recorded if this is intentionally deferred.
- Tests cover representative collisions and the temp-file hardening decision.

**Key files:** `src/model/mod.rs`, `src/render/mod.rs:269`, `src/render/mod.rs:430-473`, `src/render/templates/mcp.rs.j2:239-249`, `tests/mcp_usability.rs`.

**Dependencies:** Work item 6.

**Size:** M.

### 8. Thin rendering to workspace emission

**Goal:** Make render a file/template emission layer rather than an OpenAPI semantic analysis layer.

**Done when:**

- `src/render/mod.rs` no longer performs semantic OpenAPI traversal for MCP tools.
- `WrapperManifest` is constructed from pipeline/model data and remains render-facing only.
- Templates consume precomputed manifest/model fields only.
- Existing generated workspace layout and template output remain stable.

**Key files:** `src/render/mod.rs:27-148`, `src/model/mod.rs`, `src/render/templates/*.j2`.

**Dependencies:** Work items 6–7.

**Size:** M.

### 9. Wrap progenitor as a backend adapter

**Goal:** Isolate the current progenitor integration behind a backend seam without changing output.

**Done when:**

- The pipeline calls a backend interface implemented by a progenitor backend.
- The initial progenitor backend delegates to the existing `progenitor_driver::generate()` behavior.
- Backend diagnostics are distinguishable from spec normalization diagnostics.
- The seam does not imply support for another backend yet; it only isolates the current one.
- The adapter remains one-way for now: it receives the normalized/sliced spec and output configuration, but does not feed naming hints back into the model.

**Key files:** `src/progenitor_driver/mod.rs:12-64`, new `src/backend/mod.rs` or `src/backend/progenitor.rs`, `src/pipeline/mod.rs`.

**Dependencies:** Work item 3. This can be done before or after work items 6–8; avoid changing backend output assumptions in the same commit as render/model changes.

**Size:** M.

### 10. Name and test generated-source transforms

**Goal:** Make generated-source patches explicit, searchable technical debt.

**Done when:**

- The string-vector parser patch, complex Clap value parser patch, and unexpected-response body patch are named transforms.
- Each transform has a focused test that captures the input shape it expects and the output shape it produces.
- Tests document the coupling to progenitor/prettyplease output without broad generated-workspace rebuilds.
- The backend adapter exposes transform diagnostics when a transform is skipped or changes no source, if that matters for debugging.

**Key files:** `src/progenitor_driver/mod.rs:31-82` or the new progenitor backend module, backend transform tests.

**Dependencies:** Work item 9.

**Size:** M.

### 11. Mature verification profiles and CI shape

**Goal:** Separate fast PR checks from generated-artifact and large-spec confidence checks.

**Done when:**

- Documentation defines verification levels such as fast, standard, and deep.
- `validate` supports build validation and has a clear path for later runtime smoke validation.
- CI keeps the existing fast path and adds a manual or scheduled generated-workspace smoke workflow.
- The plan identifies which ignored tests should graduate to scheduled/manual CI first.

**Key files:** `src/cli/mod.rs`, `src/pipeline/mod.rs`, `.github/workflows/ci.yml`, ignored tests under `tests/`, docs under `docs/plans/`.

**Dependencies:** Work items 1–3; can proceed in parallel with later modeling work once pipeline validation exists.

**Size:** M.

### 12. Release metadata and dependency-status cleanup

**Goal:** Keep public docs aligned with the architecture and verification story without blocking engineering work.

**Done when:**

- README no longer contradicts current large-spec/typify status.
- CHANGELOG has a versioned release-ready entry when a release is planned.
- The typify fork status and removal condition are documented in one current place.
- Crates.io badge and release checklist are updated when publication is imminent.

**Key files:** `README.md`, `CHANGELOG.md`, `Cargo.toml`, `docs/plans/typify-patch-and-slicing-2026-05-16.md` or a newer release plan.

**Dependencies:** Work item 11 for verification language. This item should not gate pipeline/model/backend implementation.

**Size:** S.

## File-by-file Impact

| File / area | Expected direction |
| --- | --- |
| `src/cli/mod.rs` | Shrink toward argument parsing, result formatting, and command routing. Move generation orchestration and build validation out. |
| `src/pipeline/mod.rs` | New internal orchestration owner for generation request/result flow. |
| `src/spec/mod.rs` | Keep load/inspect APIs stable while carrying structured reports internally. |
| `src/spec/normalize.rs` | Split after reports exist; keep top-level normalize interface small. |
| `src/spec/slice.rs` | Keep as the model for typed options/reports and expand behavior tests. |
| `src/model/mod.rs` | New narrow API/MCP model seam. |
| `src/render/mod.rs` | Become manifest/template renderer; stop walking raw OpenAPI semantically. |
| `src/progenitor_driver/mod.rs` | Move behind backend adapter and named transforms. |
| `tests/*` | Add behavior-preserving coverage before each extraction. |
| `.github/workflows/ci.yml` | Add optional manual/scheduled generated-workspace verification after smoke tests are stable. |

## Implementation Order

Implement work items 1 through 5 in order. Work item 6 can start after work item 4 even if work item 5 is still in progress, as long as it consumes the existing normalized `openapiv3::OpenAPI` and does not depend on fully split rule modules. Work items 6 through 8 should remain separate: first construct and test the model, then add collision/safety checks, then thin render. Work items 9 and 10 should not land in the same commit as render/model changes. Work item 11 can start after work item 3 if CI/release confidence becomes the immediate priority.

## Testing Strategy

- Use existing unit tests in `src/spec/mod.rs`, `src/spec/normalize.rs`, and `src/spec/slice.rs` to guard behavior while changing internals.
- Add direct pipeline tests once the pipeline seam exists, so future refactors do not rely only on shelling out through `pp`.
- Promote a small subset of ignored smoke tests to manual or scheduled CI before making large render/backend changes.
- Keep CLI stdout/stderr behavior covered where structured reports could otherwise change user-visible output.
- Prefer fixture-sized specs for PR tests and reserve GitHub/OpenAI-scale checks for scheduled/manual verification.

## Open Questions

- Should `inspect` eventually expose structured reports by default, behind a flag, or only in a future machine-readable mode?
- Which ignored generated-workspace smoke tests should be the first to graduate to scheduled CI if runtime budget allows only one or two?

## References

- `docs/plans/long-term-architecture-2026-05-16.md`
- `docs/plans/typify-patch-and-slicing-2026-05-16.md`
- `docs/plans/mcp-agent-usability-2026-05-13.md`
- `docs/dogfood-2026-05-13.md`
- `docs/probe-oas3-2026-05-13.md`
- `docs/probe-oxide-patterns-2026-05-13.md`
- `.github/workflows/ci.yml`
- `src/cli/mod.rs`
- `src/spec/mod.rs`
- `src/spec/normalize.rs`
- `src/spec/slice.rs`
- `src/render/mod.rs`
- `src/render/templates/*.j2`
- `src/progenitor_driver/mod.rs`
- `tests/common/mod.rs`
- `tests/mcp_errors.rs`
- `tests/mcp_usability.rs`
