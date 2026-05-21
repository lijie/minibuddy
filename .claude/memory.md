# Implementation Notes (for session handoff)

## Architecture State After Phase 8

### Module Map
```
src/
├── main.rs              # load_config + create_provider + register_mcp_tools + spawn agent_task
├── config/
│   └── mod.rs           # Config/ProviderConfig/McpServerConfig + load/save
├── context/
│   ├── mod.rs           # ContextManager: estimate_tokens + truncate_if_needed
│   └── storage.rs       # JSON 会话持久化: save/load/list_sessions
├── llm/
│   ├── mod.rs           # LlmProvider trait (chat, chat_stream, chat_with_tools)
│   ├── types.rs         # Message, Role, ToolCall, ToolDefinition, LlmResponse, StreamChunk
│   ├── openai.rs        # OpenAIProvider::new(api_key, base_url, model)
│   └── anthropic.rs     # AnthropicProvider::new(api_key, model, max_tokens)
├── agent/
│   ├── mod.rs           # Agent + event_tx + context_manager + log functions
│   └── prompt.rs        # build_system_prompt()
├── tools/
│   ├── mod.rs           # Tool trait + ToolRegistry + register_mcp_tools()
│   ├── bash.rs, read_file.rs, write_file.rs, sandbox.rs
├── mcp/
│   ├── mod.rs           # re-exports
│   ├── types.rs         # JsonRpcRequest/Response/Error, McpTool, ToolCallResponse
│   ├── transport.rs     # McpTransport: spawn + call_method + shutdown
│   ├── server_manager.rs # McpServerManager (spawn+initialize+list_tools+call_tool)
│   └── tool_adapter.rs  # McpToolAdapter implements Tool trait
└── tui/
    ├── mod.rs           # App + run_app()
    ├── ui.rs            # render()
    └── event.rs         # AgentEvent, UserAction
```

### Key Invariants
- **Tool trait** 统一接口：name/description/parameters_schema/execute，内置和 MCP 工具一视同仁
- **MCP 生命周期**：spawn → initialize handshake → tools/list → 注册到 ToolRegistry → call_tool on demand
- **ContextManager** 每轮 LLM 调用前自动截断，保留 system + 最近消息
- **会话存储** `~/.mini-buddy/sessions/{timestamp}.json`
- **斜杠命令** `/save /load /new` 走 `UserAction::Command`，不进入 Agent Loop
- **日志** `crate::agent::log_info()` 写文件 `mini-buddy.log`，TUI 模式下禁止 eprintln

### MCP 实现要点
- `McpTransport::spawn(cmd, args, env, cwd)` — 进程管道 + JSON-RPC 2.0
- `McpServerManager::spawn()` 内部自动发送 `initialize` 握手（protocolVersion "2024-11-05"）
- 单 server 失败不阻断启动，日志记录警告
- `McpToolAdapter` 持有 `Arc<McpServerManager>`，多工具共享同一连接
- Drop 时 tokio::spawn fire-and-forget kill 子进程
- Serde 字段用 `#[serde(rename)]` 保持 wire 兼容（inputSchema/isError/mimeType）

### 已知遗留
- `llm/` 中有 ~10 个 dead_code warning（Phase 2 的 stream 相关代码未在 TUI 模式使用）
- `initialized` 通知未单独发送（大多数 server 仅需 initialize 响应即可工作）
- `McpServerRegistry` 已实现但未使用（留给未来热加载/卸载 server 场景）

### Phase 9+ 待做
- 流式 token 渲染到 TUI（需 chat_stream_with_tools 或混合模式）
- Markdown 渲染、语法高亮、Diff 着色
- 多行输入编辑器、滚动、主题
- Phase 11: BashTool 超时控制、Agent Loop 可中断
- ContextManager.max_tokens 可通过 config.toml 自定义（字段已在 ProviderConfig.max_tokens）
