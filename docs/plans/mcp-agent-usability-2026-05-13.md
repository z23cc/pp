# MCP Agent 可用性下一阶段计划

## Goal

把 pp 的下一阶段聚焦到真实 agent 使用体验：生成的 MCP server 已能正确调用复杂 API，下一步要让它在大工具目录和大响应体场景下更可控、更省 token，同时保持默认完整 JSON 的可信输出。

## Background

- README 已把 MCP 作为一等产物：生成 binary 支持 CLI 与 MCP stdio server，每个 OpenAPI operation 映射为一个 tool（`README.md:41`, `README.md:46-52`, `README.md:71-82`）。
- 当前 `tools/list` 接收 MCP pagination 参数但返回全部工具，DigitalOcean 这类 628-tool server 会放大工具发现问题（`src/render/templates/mcp.rs.j2:199-209`）。
- MCP 调用路径复用 CLI executor：MCP args → clap matches → `execute_operation` → `CapturedOutput` → structured MCP result（`src/render/templates/mcp.rs.j2:76-117`, `src/render/templates/cli_builder.rs.j2:58-68`）。
- `CapturedOutput` 支持单值和列表，但目前没有字段选择、压缩或截断层（`src/render/templates/context.rs.j2:20-24`, `src/render/templates/print.rs.j2:107-155`）。
- PokeAPI Claude Desktop dogfood 已验证 v0.4 runtime nullable tolerance 有效；`pokemon_retrieve` 这类端点单次返回约 270KB，说明 correctness 已过关，token/context 经济性成为下一瓶颈。
- MCP 官方工具发现是 `tools/list` + opaque cursor pagination；没有标准 `tools/search` 方法。`rmcp 1.6.0` 已有 `PaginatedRequestParams` 与 `ListToolsResult.next_cursor`，因此分页可在 generated template 层落地。参考：<https://modelcontextprotocol.io/specification/2025-11-25/server/tools>, <https://modelcontextprotocol.io/specification/2025-11-25/server/utilities/pagination>。

## Approach

做一个小而闭环的 v0.4.1：**工具目录分页 + MCP-only 响应塑形**。两者解决不同痛点：分页降低工具发现压力，响应塑形降低单次调用的 token 压力。搜索/过滤先不进入本计划；如果分页 dogfood 证明主流 client 不跟随 `nextCursor`，再把搜索作为 v0.5 重新规划。

默认行为保持不变：不传 pp 保留参数时，MCP 继续返回完整 JSON；CLI `--json` 不受影响。

## Work Items

### 1. 实现标准 `tools/list` cursor pagination

- 修改生成模板 `src/render/templates/mcp.rs.j2:199-209`，让 `list_tools(request)` 使用 `PaginatedRequestParams.cursor`。
- 使用 opaque cursor；实现上可以先用十进制 start index 字符串，如 `"0"`, `"100"`。
- 固定 server-defined page size：默认 100；如果后续 `rmcp`/MCP 类型暴露 page-size 再扩展。
- 无效 cursor 返回 MCP invalid params；最后一页省略 `nextCursor`；空 catalog 返回 `tools: []`。
- 测试：构造超过一页的 generated tools，断言第一页有 `nextCursor`，跟随 cursor 能取到后续 tools，最后一页没有 `nextCursor`。

### 2. 增加 MCP-only opt-in 响应塑形

- 在 `src/render/templates/mcp.rs.j2:76-117` 的 dispatch/capture 返回路径上处理 pp 保留参数；默认路径保持今天的完整 JSON。
- v0.4.1 只做两个参数：
  - `_pp_fields`: dot path 数组，只保留指定字段。
  - `_pp_compact`: 移除 `null`、空数组、空对象。
- `_pp_fields` v1 语法只支持对象 dot path 与整段数组保留；不支持数组通配符。示例：`name`, `types`, `stats` 可支持；`moves[].move.name` 延后。
- 只对成功 structured result 生效；错误响应保持完整诊断形态，避免削弱 `tests/mcp_errors.rs` 已覆盖的错误分类。
- 暂不移动到 `print.rs.j2` 共享层；CLI `--json` 继续做精确输出。若后续用户要求 CLI 同样支持字段选择，再单独扩展。

### 3. 保留参数冲突防护

- 在 MCP tool arg 生成阶段检测真实 API 参数是否使用 `_pp_` 前缀；相关生成 seam 是 `McpArg` 与 `add_parameter` / `add_body`（`src/render/mod.rs:52-59`, `src/render/mod.rs:313-340`, `src/render/mod.rs:350-412`）。
- 冲突策略：生成期 hard error，不做静默 rename。理由是 `_pp_` 是 MCP wrapper 的控制命名空间，静默改名会让 agent 看到与 API spec 不一致的参数。
- 测试：加入一个带 `_pp_fields` query/body 参数的微型 spec，断言 `pp generate` 失败且错误指向保留命名空间。

### 4. Dogfood 与验收

- PokeAPI：默认 `pokemon_retrieve` 仍返回完整 JSON；传 `_pp_fields=["name","types","stats"]` 后返回明显更小的结构；传 `_pp_compact=true` 后移除空值噪声。
- 一个 auth-bearing fixture（Interzoid 或 Plausible）：确认保留参数不影响 auth、上游错误分类和小响应路径。
- 大工具目录：用 synthetic spec 或 DigitalOcean 验证 `tools/list` 可分页取完；如果 Claude Desktop 不跟随 `nextCursor`，记录为 host 行为限制，不把搜索塞回 v0.4.1。
- 更新 README：说明 `tools/list` pagination、pp 保留参数、以及默认完整 JSON 不变。

## Out of Scope

- 非标准 `tools/search` 或 `_pp_search_tools` helper。本计划只记录需求，不实现。若分页对主流 MCP host 没有可见收益，再单独开 v0.5。
- `_pp_max_items` / `_pp_max_string`。先用 `_pp_fields` 解决主要 token 问题；截断类参数会引入更多边界和测试矩阵。
- CLI `--json` 字段选择。当前阶段只服务 MCP agent 使用体验。

## Risks

- 分页是协议兼容的，但部分 MCP client 可能只取第一页；需要在 README 和 dogfood 中明确观察。
- `_pp_fields` v1 不支持数组通配符，会限制某些深层裁剪场景；这是刻意缩小范围。
- 响应塑形会让 agent 更省 token，但也可能隐藏上下文；默认完整 JSON 不变是核心安全阀。

## References

- `src/render/templates/mcp.rs.j2`
- `src/render/templates/context.rs.j2`
- `src/render/templates/print.rs.j2`
- `src/render/templates/cli_builder.rs.j2`
- `src/render/mod.rs`
- `tests/mcp_errors.rs`
- `tests/dogfood.rs`
- `README.md`
- `docs/dogfood-2026-05-13.md`
- MCP tools spec: <https://modelcontextprotocol.io/specification/2025-11-25/server/tools>
- MCP pagination spec: <https://modelcontextprotocol.io/specification/2025-11-25/server/utilities/pagination>
