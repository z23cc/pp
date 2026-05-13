# Oxide progenitor edge-case probe (2026-05-13)

Source: shallow clone at `/private/tmp/probe-oxide`, commit `9c3a429`. Oxide reads one checked-in spec (`oxide.json`) and feeds it directly to progenitor: `xtask/src/main.rs:69-76`; SDK uses `generator.generate_tokens(&spec)`: `xtask/src/main.rs:89-98`; CLI uses `generator.cli(&spec, "oxide")`: `xtask/src/main.rs:117-123`.

## Multi media-type request body
- Oxide spec has this: NO. Scripted scan of `oxide.json` found `reqMulti=0`.
- How they handle: curated spec avoids it. Request media are singletons: `application/x-www-form-urlencoded` at `oxide.json:21-25`, `application/json` elsewhere, and two `application/octet-stream` bodies.
- Applicability to pp: Oxide proves “single selected media type per operation” is a valid production contract, but their solution is spec curation, not generator fallback.

## Multi content-type response
- Oxide spec has this: NO. Scan found `respMulti=0`; response media counts were `application/json=239`, `*/*=5`.
- How they handle: curated spec avoids multiple response content entries.
- Applicability to pp: descriptive evidence for normalizing to one response media type before progenitor.

## Schemaless requestBody
- Oxide spec has this: NO. Scan found `reqSchemaless=0`; the form request body has a `$ref` schema at `oxide.json:23-25`.
- How they handle: curated spec requires request schemas.
- Applicability to pp: no Oxide runtime pattern; they avoid the shape.

## typify enum collision
- Oxide spec has this: NO. Sanitized-enum collision scan found `enumCollisions=0`.
- How they handle: no evidence of pre-processing; direct `serde_json::from_reader` into progenitor at `xtask/src/main.rs:69-76`.
- Applicability to pp: not informative beyond “their spec doesn’t contain this.”

## nullable duplicate type emission
- Oxide spec has this: YES, nullable is common (`nullable=393` in scan).
- How they handle: no visible pre-processing; spec goes straight into progenitor (`xtask/src/main.rs:69-76`) and generated SDK/CLI are committed. This suggests their nullable shapes do not trigger typify#1011, or their pinned progenitor/typify path has tolerated them. Workspace pins progenitor from Oxide git and `progenitor-client = "0.14.0"` at `Cargo.toml:49-50`.
- Applicability to pp: nullable alone is not enough; pp should distinguish the specific duplicate-emission shape from generic nullable usage.

## Auth shapes other than bearer/oauth2
- Oxide spec has this: N/A in OpenAPI. Scan found `securitySchemes={}`.
- How they handle: custom CLI auth outside OpenAPI. `auth` is a hand-written command (`cli/src/cmd_auth.rs:32-46`), and runtime context stores config/credentials (`cli/src/context.rs:20-28`). Generated authenticated commands construct `Client::new_authenticated_config(ctx.client_config())` (`cli/src/main.rs:79-82`).
- Applicability to pp: Oxide does not model apikey/basic/query auth in OpenAPI; auth is product-specific wrapper code.

## OpenAPI 3.1 vs 3.0
- Oxide spec has this: OpenAPI 3.0.3. `oxide.json` declares versioned API metadata around `oxide.json:6-10`; scan read `openapi=3.0.3`.
- How they handle: no downgrade path found; xtask directly parses `oxide.json` (`xtask/src/main.rs:69-70`).
- Applicability to pp: no 3.1 handling pattern; their production path is “emit/commit 3.0.3.”

## Pagination
- Oxide spec has this: YES, list commands expose `limit`/`page_token`; generated CLI sets request builders from `limit`/`page_token` (examples at `cli/src/generated_cli.rs:17366-17684`).
- How they handle: progenitor exposes params, but Oxide hand-writes higher-level pagination only for the generic `api` command: it unwraps `PaginatedResponse { items, next_page }` (`cli/src/cmd_api.rs:233`), streams subsequent pages with `try_unfold` (`cli/src/cmd_api.rs:234-239`), and emits one JSON array (`cli/src/cmd_api.rs:223-225`).
- Applicability to pp: hand-written wrapper helpers, not progenitor auto-pagination.

## CLI command tree organization
- Oxide spec has this: generated commands plus wrapper overlay.
- How they handle: generated commands are loaded from `CliCommand::iter()` and `Cli::<OxideOverride>::get_command(op)` (`cli/src/cli_builder.rs:113-122`). Then Oxide adds hand-written commands and compound paths: `auth`, docs/version/completion, disk import, instance helpers, networking helpers, bundle/download, update upload (`cli/src/main.rs:92-119`). Some generated command args are mutated for UX, e.g. certificate file help (`cli/src/cli_builder.rs:143-150`).
- Applicability to pp: Oxide uses progenitor CLI as a substrate, then overlays product-specific grouping/renames/commands in Rust code, not spec annotations.

## Output formatting
- Oxide spec has this: N/A.
- How they handle: progenitor generates a `CliConfig` trait with formatting hooks: `success_item`, `success_no_item`, `error`, `list_start`, `list_item`, `list_end_success`, `list_end_error` (`cli/src/generated_cli.rs:21058-21077`). Oxide’s non-panicking print macros live in `cli/src/print.rs` (e.g. `print_nopipe!` handles broken pipes). The table UX is implemented via their `CliConfig` impl in `cli/src/main.rs` and generated hook calls; exact table code is larger than the hook surface, but the key seam is the generated trait.
- Applicability to pp: the reusable pattern is “generate formatting hooks, implement presentation in wrapper.”

## Patterns pp should adopt (ranked by ROI)
1. Treat progenitor output as a substrate: generate SDK/CLI, then overlay hand-written commands (`cli/src/main.rs:92-119`).
2. Normalize/curate specs before progenitor: Oxide’s production spec avoids multi-media, multi-response, schemaless body, and enum-collision cases.
3. Add explicit output hooks around generated commands (`cli/src/generated_cli.rs:21058-21077`).
4. Keep pagination as wrapper logic where needed (`cli/src/cmd_api.rs:223-239`).
5. Keep API auth and CLI login separate when the OpenAPI spec does not describe product login (`cli/src/cmd_auth.rs:32-46`).
