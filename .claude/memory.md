# Implementation Notes (for session handoff)

## Architecture State After Phase 7

### Module Map
```
src/
├── main.rs              # load_config + create_provider + spawn agent_task + handle_command
├── config/
│   └── mod.rs           # Config/ProviderConfig + load/save + resolve_api_key
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
│   ├── mod.rs           # Tool trait + ToolRegistry
│   ├── bash.rs, read_file.rs, write_file.rs, sandbox.rs
└── tui/
    ├── mod.rs           # App + run_app() + handle_key (含 / 命令路由)
    ├── ui.rs            # render() — ratatui, unicode-width, 多行内容
    └── event.rs         # AgentEvent, UserAction {Submit, Command, Quit}
```

### Key Invariants
- **ContextManager** 在每轮 LLM 调用前自动截断，保留 system + 最近消息
- **会话存储** `~/.mini-buddy/sessions/{timestamp}.json`，手动序列化 Message
- **斜杠命令** `/save /load /new` 走 `UserAction::Command`，不进入 Agent Loop
- **Token 估算** 字符级简化（中文 2:1, ASCII 4:1），max_tokens 默认 8000

### Phase 9 待做
- 流式 token 渲染、Markdown、语法高亮、滚动、多行编辑器、主题

### 后续 Phase 改动点
- Phase 8 MCP: Tool trait + ToolRegistry.register() 动态注册
- Phase 11 超时: BashTool 改 tokio::process + timeout
- ContextManager.max_tokens 可通过 config.toml 自定义（待接入）
