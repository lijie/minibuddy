# Phase 3 Implementation Notes (for future phases)

## Architecture State After Phase 3

### Key Types & Their Locations
- `LlmProvider` trait: `src/llm/mod.rs` — has 3 methods: `chat()`, `chat_stream()`, `chat_with_tools()`
- `Message` struct: `src/llm/types.rs` — has optional fields `tool_calls`, `tool_call_id`, `name`
- `ToolDefinition`, `ToolCall`, `LlmResponse`: `src/llm/types.rs`
- `Tool` trait: `src/tools/mod.rs`
- `ToolRegistry`: `src/tools/mod.rs` — has `register()`, `get()`, `definitions()`
- `Agent` struct: `src/agent/mod.rs` — holds provider + registry + messages

### Important Implementation Details

1. **Message 没有 `#[derive(Serialize)]`** — 两个 Provider 都手动用 `serde_json::json!` 构建请求体，因为 OpenAI 和 Anthropic 的 JSON 格式差异太大。添加新字段到 Message 不需要考虑 Serialize。

2. **Phase 2 代码保留但 main.rs 未使用** — `chat()`, `chat_stream()` 及 OpenAI 的 `ChatRequest`/`ChatResponse` structs 仍在代码中。Phase 5 TUI 可以复用 `chat_stream()` 做流式显示。

3. **Anthropic tool_result 合并逻辑** — `anthropic.rs` 的 `build_tool_request_body()` 会合并连续的 Role::Tool 消息。Phase 4 如果新增确认流程插入 user 消息，需注意不要打断这个合并逻辑。

4. **Agent 持有 messages 历史** — Phase 7 上下文管理需要在 `Agent.messages` 上做截断/摘要，不是在外部。

5. **工具执行是同步阻塞的** — `BashTool` 用 `std::process::Command`（同步）。Phase 11 超时控制需改为 `tokio::process::Command` + `tokio::time::timeout`。

### Phase 4 需要改动的位置
- `src/tools/bash.rs`: 在 `execute()` 中加命令预检（黑名单/权限分级）
- 新增 `src/tools/sandbox.rs`: 命令安全检查逻辑
- 新增 `src/tools/write_file.rs`: 文件写入工具（带确认流程）
- `src/agent/mod.rs`: `execute_tool()` 中可能需要插入用户确认环节

### Phase 5 需要注意
- Agent Loop 当前直接 `println!` 输出调试信息（`[Agent 思考]`、`[工具调用]`）
- TUI 引入后需要改为通过 channel 发送事件给 UI 线程
- 最终回答可以切回 `chat_stream()` 做流式输出，但需要先判断有无工具调用
