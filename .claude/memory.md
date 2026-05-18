# Implementation Notes (for session handoff)

## Architecture State After Phase 5

### Module Map
```
src/
├── main.rs              # channel 创建 + spawn agent_task + tui::run_app()
├── llm/
│   ├── mod.rs           # LlmProvider trait (chat, chat_stream, chat_with_tools)
│   ├── types.rs         # Message, Role, ToolCall, ToolDefinition, LlmResponse, StreamChunk
│   ├── openai.rs        # OpenAI/DeepSeek/Ollama provider
│   └── anthropic.rs     # Anthropic Claude provider
├── agent/
│   ├── mod.rs           # Agent struct (provider + registry + messages + event_tx)
│   └── prompt.rs        # build_system_prompt()
├── tools/
│   ├── mod.rs           # Tool trait + ToolRegistry + create_default_registry()
│   ├── bash.rs          # BashTool
│   ├── read_file.rs     # ReadFileTool
│   ├── write_file.rs    # WriteFileTool
│   └── sandbox.rs       # classify() → PermissionLevel
└── tui/
    ├── mod.rs           # App struct + run_app() event loop (tokio::select!)
    ├── ui.rs            # render() — ratatui layout + widgets
    └── event.rs         # AgentEvent, UserAction, ChatRole, ChatEntry, InputMode
```

### Communication Architecture
```
TUI task (main async)  ←→  Agent task (tokio::spawn)
  user_tx → user_rx: UserAction {Submit, Quit}
  agent_rx ← agent_tx: AgentEvent {ThinkingStarted, ToolCallStart, FinalResponse, ConfirmationRequest...}
  Confirmation: oneshot embedded in AgentEvent::ConfirmationRequest
```

### Key Invariants
- **Agent::run() returns Result<()>** — 最终回答通过 `AgentEvent::FinalResponse` 发送
- **确认仍不进入消息历史** — oneshot 交互对 LLM 透明
- **日志写入 `mini-buddy.log`** — `log()`, `log_messages()`, `log_response()` in agent/mod.rs
- **unicode-width** 用于光标位置计算（中文字符占 2 列）

### Phase 9 待做（从 Phase 5 推迟）
- 流式 token 渲染（需 chat_stream_with_tools 或混合模式）
- Markdown 渲染、语法高亮、Diff 着色
- 多行输入编辑器、上下滚动
- Spinner/进度动画、主题系统

### 后续 Phase 改动点
- Phase 7 上下文管理: 操作 `Agent.messages` 做截断/摘要
- Phase 8 MCP: Tool trait + ToolRegistry.register() 动态注册
- Phase 11 超时: BashTool 改 tokio::process::Command + timeout
