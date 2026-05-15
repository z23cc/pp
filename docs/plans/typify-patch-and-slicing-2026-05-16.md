# typify patch verification + spec slicing design — 2026-05-16

## Summary

`oxidecomputer/typify#1012` is worth carrying temporarily, but it is **not sufficient by itself** for GitHub-scale specs. The implemented fix is a layered approach:

1. Keep the temporary typify fork patch while upstream is open.
2. Fix nullable-composition at the typify source: `oneOf: [null, { oneOf: [...] }]` now names the inner composed type as `NameInner` instead of recursively reusing `Name`.
3. Patch generated Clap parsers for complex `types::...` arguments to parse from JSON/string values instead of requiring `ValueParserFactory`.
4. Render MCP input schemas from JSON strings at runtime instead of expanding huge `serde_json::json!` literals at compile time.
5. Add operation/tag/path-prefix slicing so users can still generate smaller, faster, isolated API surfaces.

With these pp fixes plus the typify source fix, the full GitHub REST spec now generates and `cargo build --release` succeeds without dropping nullable-composition semantics in pp.

## Patch under test

`Cargo.toml` now patches the transitive typify crates to the PR branch:

- repo: `https://github.com/z23cc/typify`
- source branch: `fix/nullable-composition-option`
- pinned rev: `1e4213a8e76f2bcc54ba1f70c04816aa388b5f08`
- patched crates: `typify`, `typify-impl`, `typify-macro`

Cargo resolution confirmed with:

```bash
cargo tree -i typify-impl
```

## Verification results

### Existing pp suite

Command set:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Result:

- `cargo test`: 35 passed, 9 ignored
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo fmt --check`: pass

### GitHub REST OpenAPI generation

Spec source:

- sparse clone of `github/rest-api-description`
- file: `/tmp/pp-wild/rest-api-description/descriptions/api.github.com/api.github.com.yaml`
- size: ~9.1 MB
- operations reported by pp: 1183

Generate command:

```bash
cargo run --quiet -- generate \
  /tmp/pp-wild/rest-api-description/descriptions/api.github.com/api.github.com.yaml \
  -o /tmp/pp-wild/github-patched-out
```

Result:

- `pp generate`: pass in ~14s
- generated workspace: `/tmp/pp-wild/github-patched-out`

Notable normalization tail:

- dropped defaults from 1104 schemas
- dropped 1 unsupported request-body operation: `markdown/render-raw`
- relaxed 5228 response schemas

### Generated workspace build baseline before local pp fixes

Command:

```bash
cd /tmp/pp-wild/github-patched-out
cargo build --release
```

Result:

- exit: 101
- duration: ~115s
- final: `could not compile git-hub-rest-api-api due to 31 previous errors; 2379 warnings emitted`

Error histogram after stripping ANSI:

| Code | Count | Meaning |
|---|---:|---|
| E0072 | 1 | recursive type has infinite size |
| E0119 | 1 | conflicting `From` impl |
| E0599 | 27 | generated Clap value parser bounds not satisfied |
| E0391 | 2 | layout/drop-check cycle from recursive type |

Remaining recursive type:

```rust
pub struct NullableSecretScanningFirstDetectedLocation(
    pub ::std::option::Option<NullableSecretScanningFirstDetectedLocation>,
);
```

Input shape:

```yaml
nullable-secret-scanning-first-detected-location:
  oneOf:
    - $ref: '#/components/schemas/secret-scanning-location-commit'
    # ... more location refs ...
  nullable: true
```

Interpretation:

- #1012 reduces the originally reported nullable object duplicate/self-recursive class, but a `nullable + oneOf` named component still self-recurses.
- The remaining `E0599` errors are a separate generated CLI issue: complex enum/newtype parameter types are handed to `clap::value_parser!` even though they do not satisfy the required parser traits.
- Therefore the original typify patch is useful but incomplete; the fork now also fixes nullable-composition option inner naming at the source.

### Current verification after implementation

Commands:

```bash
cargo fmt --check
cargo test --quiet
cargo clippy --all-targets -- -D warnings

cargo run --quiet -- generate \
  /tmp/pp-wild/rest-api-description/descriptions/api.github.com/api.github.com.yaml \
  -o /tmp/pp-wild/github-full-fixed-out --build

cargo run --quiet -- generate \
  /tmp/pp-wild/rest-api-description/descriptions/api.github.com/api.github.com.yaml \
  -o /tmp/pp-wild/github-meta-final-out --include-tag meta --build
```

Results:

- local suite: `cargo test` 36 passed, 9 ignored; fmt and clippy pass.
- full GitHub REST spec: 1183 operations, generated workspace built successfully with pinned typify rev `1e4213a8e76f2bcc54ba1f70c04816aa388b5f08`.
- normalization tail: 1104 dropped defaults, 1 unsupported raw-markdown operation dropped, 5228 response schemas relaxed; pp no longer drops nullable from composed schemas.
- generated wrapper: no remaining `json!` recursion-limit failure after parsing MCP input schemas from string literals.
- meta slice: kept 5 operations, pruned schemas 926 -> 2, build succeeded in ~28s.

## Why slicing is now required

Full-spec generation couples unrelated operations, schemas, and CLI parser surfaces into one giant Rust module. A single unsupported schema or parameter type blocks the whole API.

Slicing changes the failure mode:

- users can generate the subset they need;
- CI can dogfood representative slices instead of only all-or-nothing builds;
- later multi-crate output can compose slices without forcing one huge type universe.

## Slicing UX

Add flags to `pp generate` and `pp inspect`:

```text
--include-operation <OPERATION_ID>   repeatable exact match
--include-tag <TAG>                  repeatable exact match
--include-path-prefix <PREFIX>       repeatable string prefix
--exclude-operation <OPERATION_ID>   repeatable exact match, applied last
--list-operations                    inspect helper; prints stable JSONL rows: id, method, path, tags
```

Selection semantics:

1. If no include flag is present, start with all operations.
2. If any include flag is present, select operations matching **any** include predicate.
3. Apply excludes after includes.
4. Drop paths that have no selected operations.
5. Recompute facts after slicing: operation count, auth inference, MCP tool list, bin name unchanged unless user passes `--name`.

Examples:

```bash
pp inspect github.yaml --list-operations
pp generate github.yaml -o out/github-meta --include-tag meta --build
pp generate github.yaml -o out/github-repos --include-path-prefix /repos/ --build
pp generate github.yaml -o out/github-core \
  --include-operation meta/get --include-operation rate-limit/get --build
```

## Internal design

### Data types

Add `src/spec/slice.rs`:

```rust
pub struct SliceOptions {
    pub include_operations: Vec<String>,
    pub include_tags: Vec<String>,
    pub include_path_prefixes: Vec<String>,
    pub exclude_operations: Vec<String>,
}

pub struct SliceReport {
    pub kept_operations: usize,
    pub dropped_operations: usize,
    pub pruned_components: PrunedComponents,
}
```

`SliceOptions::is_noop()` keeps current behavior exactly.

### Pipeline

Current generate pipeline:

```text
parse -> pre-normalize -> typed normalize -> inspect facts -> render manifest -> progenitor
```

Target pipeline:

```text
parse -> pre-normalize -> typed normalize -> slice/prune -> inspect facts -> render manifest -> progenitor
```

Implementation options:

- either change `spec::load(path)` to `spec::load_with_options(path, LoadOptions)`;
- or keep `load(path)` as default wrapper around `load_with_options(path, LoadOptions::default())`.

Prefer the second option to minimize existing test churn.

### Operation filtering

For each path item:

- evaluate each method operation independently;
- set unselected operation fields to `None`;
- preserve path-level parameters only when at least one operation remains;
- remove the whole path if no method operation remains.

Use the existing operation walking style in `normalize.rs` as a model, but centralize method slots in a small helper to avoid eight copy-pasted blocks.

### Component pruning

Phase 1 should implement conservative pruning, not perfect minimization.

Roots:

- selected operations;
- selected path-level parameters;
- global security schemes and operation-level security schemes;
- request bodies, parameters, responses, headers, and schemas referenced by selected operations.

Reference walker should cover:

- `ReferenceOr<T>` values;
- parameter schema/content;
- request/response content schemas;
- schema properties;
- `additionalProperties`;
- array items;
- `allOf`, `oneOf`, `anyOf`, `not`;
- response headers if present.

Prune maps only after the graph walk:

- `components.schemas`
- `components.parameters`
- `components.request_bodies`
- `components.responses`
- `components.headers` if needed

Examples, links, callbacks, and extensions may be retained only if their `$ref` dependencies are also retained. If they introduce dangling refs or pull in large unrelated schema graphs, prune them in v1 rather than leaving them as hidden roots.

### Diagnostics

Emit stderr warnings like existing normalization:

```text
pp: sliced spec — kept 42 operations, dropped 1141 operations
pp: pruned components — schemas 5193 -> 312, responses 900 -> 41, parameters 210 -> 18
```

For an empty slice, fail early:

```text
no operations matched slice filters; use `pp inspect --list-operations` to discover operation IDs/tags
```

## Acceptance plan

### Unit tests

Add a fixture with:

- two tags;
- shared path-level parameter;
- shared component schema;
- a component schema only referenced by the dropped operation.

Assertions:

- selected operation remains;
- unselected operation is removed;
- unused component is pruned;
- shared component remains;
- `mcp_tools` only contains selected operations.

### Integration tests

1. Existing smoke tests pass with default no-op slicing.
2. Prune unreachable components before generation; a selected tiny slice must not retain unrelated schemas such as `nullable-secret-scanning-first-detected-location`.
3. Preserve effective security semantics: selected operations must keep inherited root security, operation overrides, and explicit `security: []` behavior while pruning unused `securitySchemes`.
4. GitHub REST small slice builds:

```bash
pp generate api.github.com.yaml -o /tmp/pp-wild/github-meta \
  --include-tag meta --build
```

5. A path-prefix slice builds or produces an actionable unsupported-shape error:

```bash
pp generate api.github.com.yaml -o /tmp/pp-wild/github-repos \
  --include-path-prefix /repos/ --build
```

6. Empty slice fails with the discovery hint.

## Follow-up fixes outside slicing

1. `nullable + oneOf` typify/progenitor failure:
   - implemented typify fork fix: option detection now converts the non-null branch with `NameInner` when the nullable wrapper has a required name.
   - pp fallback that dropped `nullable` from composed schemas has been removed.
   - upstream should still receive this as a typify PR/review.

2. Clap parser failures for complex query parameters:
   - implemented generated-source patch: complex `types::...` and `Vec<types::...>` parsers accept JSON strings/JSON arrays/comma-separated values.
   - longer term: upstream progenitor CLI generation should expose an official parser hook.

3. MCP schema compile-time recursion:
   - implemented template fix: parse pre-serialized schema strings with `serde_json::from_str` instead of using huge `json!` macro invocations.

4. Multi-crate slicing:
   - after single-slice generation lands, add `--split-by-tag` to produce one API crate per tag plus a wrapper dispatcher.

## Decision

Keep the temporary typify fork patch because it now fixes both the prior nullable duplicate-type issue and the nullable-composition `Option<Self>` issue. Keep pp's slicing, Clap parser patch, and MCP schema rendering fix until upstream typify/progenitor can natively handle the whole GitHub-scale path.
