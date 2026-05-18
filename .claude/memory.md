# Implementation Notes (for session handoff)

## Architecture State After Phase 4

### Module Map
```
src/
├── main.rs              # 入口: create_provider() + Agent 初始化 + 对话循环
├── llm/
│   ├── mod.rs           # LlmProvider trait (chat, chat_stream, chat_with_tools)
│   ├── types.rs         # Message, Role, ToolCall, ToolDefinition, LlmResponse, StreamChunk
│   ├── openai.rs        # OpenAI/DeepSeek provider
│   └── anthropic.rs     # Anthropic Claude provider
├── agent/
│   ├── mod.rs           # Agent struct (provider + registry + messages) + execute_tool() 含权限检查
│   └── prompt.rs        # build_system_prompt()
└── tools/
    ├── mod.rs           # Tool trait + ToolRegistry + create_default_registry()
    ├── bash.rs          # BashTool — sh -c 执行
    ├── read_file.rs     # ReadFileTool — std::fs::read_to_string
    ├── write_file.rs    # WriteFileTool — std::fs::write + 系统路径拦截
    └── sandbox.rs       # classify() → PermissionLevel {Read, Write, Dangerous}
```

### Key Invariants
- **Message 无 `#[derive(Serialize)]`** — Provider 手动构建 JSON
- **Anthropic 连续 tool_result 会合并**为单个 user 消息 (`anthropic.rs` `build_tool_request_body`)
- **用户确认不进入消息历史** — 纯终端 I/O，LLM 只看到 tool_result
- **工具执行仍是同步** `std::process::Command` — Phase 11 改 tokio::process

### Phase 5 (TUI) 改动点
- `agent/mod.rs` `ask_user_confirmation()`: 改为 async channel
- `agent/mod.rs` `run()` 中的 `println!`: 改为事件发送
- 最终回答可用 `chat_stream()` 做流式渲染

### Phase 5 之后
- Phase 7 上下文管理: 操作 `Agent.messages` 做截断/摘要
- Phase 8 MCP: 新工具实现 `Tool` trait，通过 `ToolRegistry.register()` 动态注册
- Phase 11 超时: BashTool 改 `tokio::process::Command` + `tokio::time::timeout`
