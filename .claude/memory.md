# Implementation Notes (for session handoff)

## Architecture State After Phase 6

### Module Map
```
src/
├── main.rs              # load_config() + create_provider(&cfg) + spawn agent + run TUI
├── config/
│   └── mod.rs           # Config/ProviderConfig structs + load/save + resolve_api_key()
├── llm/
│   ├── mod.rs           # LlmProvider trait (chat, chat_stream, chat_with_tools)
│   ├── types.rs         # Message, Role, ToolCall, ToolDefinition, LlmResponse, StreamChunk
│   ├── openai.rs        # OpenAIProvider::new(api_key, base_url, model)
│   └── anthropic.rs     # AnthropicProvider::new(api_key, model, max_tokens)
├── agent/
│   ├── mod.rs           # Agent struct + event_tx + log functions + execute_tool 权限检查
│   └── prompt.rs        # build_system_prompt()
├── tools/
│   ├── mod.rs           # Tool trait + ToolRegistry + create_default_registry()
│   ├── bash.rs          # BashTool
│   ├── read_file.rs     # ReadFileTool
│   ├── write_file.rs    # WriteFileTool
│   └── sandbox.rs       # classify() → PermissionLevel
└── tui/
    ├── mod.rs           # App struct + run_app() + tokio::select! event loop
    ├── ui.rs            # render() — ratatui widgets, unicode-width 光标
    └── event.rs         # AgentEvent, UserAction, ChatRole, ChatEntry, InputMode
```

### Config System
- 配置路径: `~/.mini-buddy/config.toml`
- 优先级: 环境变量 > 配置文件 > 内置默认值
- `Config::active_provider_name()` 检查 `LLM_PROVIDER` env var
- `ProviderConfig::resolve_api_key()` 检查 `api_key_env` 指定的 env var
- Provider 通过 `type` 字段分发 ("openai" | "anthropic")

### Key Invariants
- **Agent::run() → Result<()>**, 最终回答通过 `AgentEvent::FinalResponse` 发送
- **确认不入消息历史** — oneshot 交互对 LLM 透明
- **日志写入 `mini-buddy.log`** — log(), log_messages(), log_response()
- **unicode-width** 用于 TUI 光标位置（中文 2 列宽）
- **Provider 快捷构造器** (deepseek/ollama/claude_sonnet) 仍存在但已未使用，create_provider 直接用 ::new()

### Phase 9 待做（从 Phase 5 推迟）
- 流式 token 渲染、Markdown、语法高亮、Diff 着色
- 多行输入编辑器、上下滚动、Spinner、主题

### 后续 Phase 改动点
- Phase 7 上下文管理: Agent.messages 截断/摘要
- Phase 8 MCP: Tool trait + ToolRegistry.register() 动态注册
- Phase 11 超时: BashTool 改 tokio::process + timeout
