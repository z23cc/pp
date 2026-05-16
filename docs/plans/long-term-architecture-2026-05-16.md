# pp long-term architecture plan — 2026-05-16

## Problem Statement

`pp` is evolving from a thin OpenAPI-to-Rust wrapper into a compiler for real-world OpenAPI specifications. The project already solves valuable problems: tolerant spec loading, normalization, operation slicing, progenitor integration, generated CLI wrappers, and MCP server generation.

The long-term risk is that these responsibilities remain concentrated in shallow modules. Each new real-world spec failure currently tends to add another special case near parsing, normalization, rendering, or generated-code patching. If this continues, `pp` may become capable but difficult to reason about, difficult to test, and fragile when upstream crates change.

The developer-facing problem is therefore not only “support more specs.” It is: make `pp` a maintainable compiler platform where compatibility work is explicit, reportable, testable, and isolated behind deep interfaces.

## Long-term Thesis

The durable value of `pp` should be:

1. **Spec tolerance** — accept messy vendor OpenAPI documents and explain what was changed.
2. **Generation reliability** — turn a selected API surface into a reproducible Rust workspace or fail with actionable diagnostics.
3. **Agent-native runtime** — make generated CLIs and MCP servers efficient, inspectable, and predictable for both humans and agents.

The long-term architecture should treat OpenAPI input as untrusted source code. `pp` should behave like a compiler pipeline, not a collection of ad hoc transformations.

## Target Architecture

Move toward a staged pipeline with explicit intermediate artifacts:

1. **RawSpec** — bytes plus source metadata.
2. **TolerantSpec** — pre-deserialization repairs applied to known real-world invalid shapes.
3. **ParsedSpec** — typed OpenAPI representation.
4. **NormalizedSpec** — progenitor-compatible semantic normalization, with a structured report.
5. **SlicedSpec** — operation subset plus pruned reachable components.
6. **ApiModel** — stable `pp` intermediate model for operations, auth, schemas, naming, and runtime behavior.
7. **WorkspacePlan** — backend-independent description of files/crates/templates to emit.
8. **GeneratedWorkspace** — rendered files plus generated API crate.
9. **VerifiedArtifact** — generated workspace after optional build/smoke validation.

Each stage should have a small interface, typed errors, and a report. Callers should not need to know implementation details from previous stages.

## Current Implementation Status

As of the 2026-05-16 follow-up pass, `pp` has implemented the main internal seams this plan called for: generation runs through an internal pipeline request/result path; spec preparation emits structured reports and transform plans; normalization is split into rule groups; selected transforms include machine-readable audit metadata; MCP tool data is built by the model layer before rendering; rendering consumes a wrapper manifest; Progenitor is isolated behind a backend seam with named source-transform diagnostics; and verification profiles are documented.

Remaining work is narrower than the original plan. The generated MCP runtime now treats `runtime.mcp_invocation.progenitor_cli_bridge` as an explicit Progenitor runtime adapter contract rather than hidden local debt. Direct typed operation invocation remains blocked until generated output exposes stable operation metadata. OAuth2 is still modeled as bearer-token input only, and validation remains build-focused rather than a full runtime-smoke validation framework.

## Solution

### Deepen the pipeline module

Create a pipeline module that owns generation orchestration. The CLI should translate user input into a generation request and then call one public interface. This gives the codebase a seam for tests, future library usage, batch generation, and richer validation modes.

Expected leverage:

- The CLI stops being the coordinator for every generation detail.
- Tests can call the generation pipeline without shelling out through the binary.
- Future commands can reuse the same interface instead of duplicating orchestration.

### Turn normalization into a rule system

Normalization should become a collection of named rules, not an expanding blob of transformations. Each rule should declare:

- a stable rule id;
- what shape it detects;
- what it changes;
- whether it is semantic-preserving, lossy, or compatibility-only;
- what warning/report entry it emits;
- which fixture guards it.

Expected leverage:

- Users can inspect exactly what `pp` changed.
- Real-world failures can be fixed by adding isolated rules.
- Rules can be enabled, disabled, or audited later.

### Introduce a stable API model

Rendering should consume a `pp` model instead of walking raw OpenAPI directly. The model should describe operations, arguments, request bodies, response expectations, auth behavior, generated CLI names, MCP tool schemas, and wrapper-specific controls.

Expected leverage:

- Templates become simpler and less semantic.
- CLI and MCP behavior can be tested before rendering.
- Operation naming, body flattening, collision detection, and schema metadata can be handled in one locality.

### Treat progenitor as a backend adapter

The progenitor integration should be contained behind a backend interface. Source patches to generated code should be represented as explicit named transforms with version assumptions and tests.

Expected leverage:

- Upstream changes become easier to diagnose.
- Future backends or modes become possible without rewriting the whole project.
- Backend-generated source transforms become visible adapter constraints with version assumptions instead of hidden coupling.

### Make verification first-class

`pp` should distinguish generation success from artifact validity. A generated workspace is not trustworthy until it has passed selected validation.

Expected leverage:

- `validate` becomes meaningful.
- CI can test generated artifacts on a schedule without slowing every PR.
- Users get a clearer answer: parse succeeded, normalize succeeded, generate succeeded, build succeeded, or runtime smoke succeeded.

## Commits

The plan should be implemented in small commits that leave the project working after every step.

### Phase 1 — Make current behavior safer

1. Add a CI job or workflow mode for selected generated-workspace smoke tests.
2. Add a smoke test for operation slicing with include-by-operation, include-by-tag, include-by-path-prefix, exclude, and operation listing.
3. Add one focused test for MCP argument name collisions between parameters and flattened body fields.
4. Add one focused test for generated MCP temporary body file uniqueness under repeated calls.
5. Implement the existing validate command as a simple generated-workspace build check.
6. Document the temporary typify patch and the release condition for removing it.

### Phase 2 — Extract the pipeline seam

1. Add a generation request type that represents all user-configurable generation options.
2. Add a generation result type that contains derived facts, normalization warnings, emitted workspace path, and optional build result.
3. Move orchestration from the CLI into a pipeline interface without changing behavior.
4. Change the CLI to call the pipeline and preserve existing stderr/stdout behavior.
5. Add direct unit or integration tests for the pipeline interface.
6. Keep existing CLI smoke tests as end-to-end guards.

### Phase 3 — Introduce structured reports

Status: implemented internally. `pp inspect --reports` exposes facts plus structured preparation reports, and generated workspaces include `pp-transform-plan.json` with approval and audit data.

1. Replace loose normalization warning strings internally with structured report entries.
2. Preserve the current human-readable warning output by formatting report entries at the boundary.
3. Include rule id, severity, affected operation/schema when available, and short message.
4. Add JSON inspect support for reporting normalization decisions.
5. Add regression tests for report stability on bundled fixtures.

### Phase 4 — Split normalization into rule groups

1. Extract pre-deserialization tolerance rules.
2. Extract OpenAPI version downgrade rules.
3. Extract progenitor compatibility rules.
4. Extract response relaxation rules.
5. Extract operation naming rules.
6. Add rule-level tests for each extracted group.
7. Keep a top-level normalization interface that applies the default rule set.

### Phase 5 — Build the API model

1. Add model types for API facts, auth model, operation model, parameter model, request body model, response model, and schema model.
2. Build the model from the normalized and sliced spec without changing rendering yet.
3. Add tests that assert model output for bundled fixtures.
4. Move MCP tool construction to consume the operation model.
5. Move CLI/runtime naming decisions to the model layer.
6. Keep templates consuming a compatibility manifest until all fields are migrated.

### Phase 6 — Thin the rendering layer

1. Change templates to consume precomputed model data only.
2. Remove raw OpenAPI traversal from rendering.
3. Add rendered-file snapshot or structural tests for representative generated workspaces.
4. Add schema metadata preservation for MCP input schemas where safe.
5. Add explicit collision errors for ambiguous flattened body fields.

### Phase 7 — Isolate backend adapters

Status: implemented for the current Progenitor backend seam, named source transforms, and explicit MCP CLI bridge adapter audit. Direct typed invocation from generated MCP tools remains future runtime/backend work because current generated Progenitor output does not expose stable operation metadata.

1. Define a backend interface for API crate generation.
2. Move progenitor-specific settings and source transforms behind the progenitor adapter.
3. Name each generated-code transform and add tests for the input/output shape it expects.
4. Emit backend diagnostics separately from spec normalization diagnostics.
5. Document which failures belong upstream and which belong in `pp`.

### Phase 8 — Mature verification and release flow

1. Expand validate from build-only to configurable validation levels.
2. Add fast, standard, and deep verification profiles.
3. Add scheduled large-spec regression runs for at least one large public spec and selected slices.
4. Replace placeholder release metadata.
5. Update changelog from unreleased notes into versioned release entries.
6. Publish a release checklist that includes generated artifact verification.

## Decision Document

- `pp` should be modeled as a compiler pipeline, not a CLI script.
- The pipeline interface should become the primary seam for generation behavior.
- Normalization decisions should be structured and inspectable.
- Rendering should not own OpenAPI semantic traversal long term.
- MCP tool modeling should live in the API model layer, not in templates.
- Progenitor should be treated as an adapter behind a backend seam.
- Generated workspace validation should be a first-class result, not an optional afterthought.
- The project should optimize first for reliable Agent-facing MCP generation, while preserving human CLI usefulness.

## Testing Decisions

Good tests should assert externally visible behavior and stable intermediate contracts, not incidental implementation details.

Test layers:

1. **Rule tests** — each normalization rule has minimal input and expected report entries.
2. **Model tests** — representative specs produce expected operation/auth/schema models.
3. **Pipeline tests** — generation requests produce workspaces and reports without requiring CLI shelling.
4. **CLI tests** — command-line behavior remains stable for users.
5. **Generated artifact tests** — selected generated workspaces build and selected runtime calls work.
6. **MCP tests** — tools/list, tools/call, auth errors, response shaping, and schema generation stay stable.
7. **Large-spec scheduled tests** — expensive regressions run outside normal PR latency.

Prior art already exists in the smoke tests, MCP usability tests, auth tests, and dogfood documentation. The long-term plan should promote selected ignored tests into scheduled or manual CI workflows.

## Out of Scope

- Replacing the OpenAPI parser as the first move.
- Rewriting the progenitor backend before the pipeline and model seams exist.
- Supporting every OpenAPI edge case immediately.
- Adding non-standard MCP search before pagination and response shaping are proven insufficient.
- Designing a full OAuth flow in the architecture refactor.
- Publishing a stable library API before the internal pipeline interface settles.

## Milestones

### Milestone A — Safer MVP

`pp` has CI coverage for generated workspaces, a working validate command, and slicing smoke coverage.

### Milestone B — Compiler-shaped core

Generation goes through a pipeline interface with structured reports. The CLI is no longer the orchestration owner.

### Milestone C — Stable model layer

Rendering and MCP tool construction consume `pp` model types instead of raw OpenAPI traversal.

### Milestone D — Backend isolation

Progenitor-specific code and generated-source transforms are isolated, named, and tested. The remaining backend/runtime constraint is direct typed invocation for MCP calls; today that path is explicitly audited as a Progenitor CLI bridge adapter, and replacement depends on generated operation metadata.

### Milestone E — Release-grade verification

`pp` has standard validation profiles, scheduled large-spec regressions, and a clear release checklist.

## Risks

- Over-abstracting too early could slow down real compatibility work.
- A model layer that mirrors OpenAPI too closely will not create leverage.
- A model layer that is too opinionated may block valid specs.
- Large-spec CI can become expensive; it should be scheduled or manual unless a fast slice is enough.
- Maintaining a typify patch long term increases release risk.

## Open Questions

1. Should normalization rules be configurable by users, or only reported?
2. Should destructive compatibility rules require explicit opt-in in a future strict mode?
3. What is the minimum stable API model needed before moving rendering onto it?
4. Should `pp` eventually support an MCP-only output mode without a human CLI wrapper?
5. Which public specs should become the canonical compatibility corpus?

## Recommended Next Step

Start with Phase 1 and Phase 2. They reduce regression risk and create a deep pipeline seam without forcing a large rewrite. Once the pipeline seam exists, the normalization rule system and API model can be introduced incrementally.
