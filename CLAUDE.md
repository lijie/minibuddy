# mini-buddy

## 项目概述

mini-buddy 是一个极简架构的轻量级 AI 编程智能体（教学项目），使用 Rust 构建。目标是剖析、学习和复刻大厂 Agent 架构。

## 当前进度

- ✅ Phase 0: 项目骨架（Cargo 初始化，基础依赖配置）
- ✅ Phase 1: 最简 LLM 调用（单轮问答，OpenAI 兼容接口）
- ✅ Phase 2: 多轮对话 + 流式输出
- ✅ Phase 3: Agent Loop + Tool Use
- ✅ Phase 4: Bash 沙盒 + 权限控制
- ✅ Phase 5: TUI 基础界面（Ratatui）
- ✅ Phase 6: 配置系统（TOML）
- ✅ Phase 7: 上下文管理 + 会话持久化
- ✅ Phase 8: MCP（Model Context Protocol）- 外部工具集成
- ⬜ Phase 9-12: 其他高级特性

完整计划详见 `docs/plan.md`

## 架构分层

```
TUI Layer → Agent Loop → Tool Registry → LLM Provider → Context/State Manager
                              ↓
                    [Built-in Tools] + [MCP Tools]
```

## Phase 8: MCP 支持完成情况

### 实现内容

✅ **Phase 8a: 配置扩展** - MCP 服务器配置
- McpServerConfig 结构体：command, args, env, cwd, startup_timeout_secs
- 配置文件支持 [mcp.*] 段

✅ **Phase 8b: 传输层** - JSON-RPC 通信
- McpTransport 类型：进程管理、stdin/stdout 通信
- JSON-RPC 2.0 协议实现
- 请求 ID 匹配和响应处理

✅ **Phase 8c: 服务器管理** - 进程生命周期
- McpServerManager：启动、工具发现、工具调用
- McpServerRegistry：多个 MCP 服务器管理
- 工具缓存优化

✅ **Phase 8d: 工具适配器** - 工具集成
- McpToolAdapter：MCP 工具包装为 Tool trait
- 与 ToolRegistry 无缝集成
- register_mcp_tools() 函数自动发现和注册

✅ **Phase 8e: 端到端集成** - 完整工作流
- main.rs 中的 MCP 初始化
- 错误处理和日志记录
- 54 项测试全部通过

### 关键特性

1. **自动发现**: 启动时自动发现 MCP 服务器的工具
2. **无缝集成**: MCP 工具与内置工具在 Agent 中一视同仁
3. **并发支持**: 多个 MCP 服务器并行运行
4. **错误处理**: 单个服务器失败不影响其他服务器和应用启动
5. **配置灵活**: 支持环境变量、工作目录、启动超时配置

### 配置示例

```toml
[mcp.filesystem]
command = "mcp-filesystem"
args = []

[mcp.web-browser]
command = "mcp-browser"
args = ["--timeout", "30"]
env = { BROWSER_UA = "mini-buddy/1.0" }
```

### 文件位置

- **配置**: `~/.mini-buddy/config.toml`
- **MCP 模块**: `src/mcp/` (types, transport, server_manager, tool_adapter)
- **工具注册**: `src/tools/mod.rs` (register_mcp_tools function)
- **使用指南**: `docs/MCP-USAGE-GUIDE.md`

### 测试覆盖

- 43 单元测试（MCP 模块 + 工具沙盒）
- 6 集成测试（MCP 配置和流程）
- 5 配置测试（TOML 序列化）
- **总计**: 54 项测试全部通过

## 技术栈

- **语言**: Rust (edition 2024)
- **异步运行时**: tokio
- **HTTP**: reqwest (with stream feature)
- **序列化**: serde + serde_json
- **错误处理**: anyhow + thiserror
- **TUI** (Phase 5+): ratatui + crossterm
- **MCP 协议**: JSON-RPC 2.0 over stdio (Phase 8)

## 项目结构

```
src/
├── main.rs          # 入口
├── llm/             # (Phase 1) LLM Provider 层
├── agent/           # (Phase 3) Agent Loop
├── tools/           # (Phase 3) 工具注册与执行
├── tui/             # (Phase 5) 终端 UI
├── config/          # (Phase 6) 配置系统
├── context/         # (Phase 7) 上下文管理
└── mcp/             # (Phase 8) MCP 协议
    ├── types.rs         # JSON-RPC 类型定义
    ├── transport.rs     # 进程通信层
    ├── server_manager.rs # 服务器和工具管理
    └── tool_adapter.rs  # MCP → Tool trait 适配

tests/
├── test_config_with_mcp.rs  # 配置测试
└── integration_mcp.rs        # 集成测试

docs/
├── MCP-USAGE-GUIDE.md        # MCP 使用文档
└── README.md                 # 架构文档
```

## 开发约定

- 每完成一个 Phase 打 git tag（如 `v0.1-phase8`）
- 先用裸终端跑通逻辑（Phase 1-4），再引入 TUI（Phase 5）
- LLM Provider 统一用 trait 抽象，方便多模型切换
- Tool 也统一用 trait 抽象，支持任意工具类型（Phase 8 的 MCP 工具）
- 教学项目，关键设计决策处写注释说明"为什么这样做"
- 中文注释和文档

## 构建与运行

```bash
cargo build
cargo run
cargo test   # 运行所有测试
```

## 环境变量（Phase 1 开始需要）

```bash
export DEEPSEEK_API_KEY="your-key"      # DeepSeek
export ANTHROPIC_API_KEY="your-key"     # Anthropic Claude
export OPENAI_API_KEY="your-key"        # OpenAI 兼容接口
```

## 关键设计参考

- **Agent Loop**: 感知→思考→行动循环，支持 CoT 和 reasoning_content
- **Tool Use**: OpenAI function calling / Anthropic tool use 格式
- **安全**: Bash 命令黑名单 + 权限分级（读自动/写确认/危险拦截）
- **异步**: UI 线程与 Agent 线程通过 mpsc channel 通信（Phase 5+）
- **工具扩展**: 统一的 Tool trait，支持内置工具和 MCP 工具（Phase 8）
- **配置管理**: TOML 配置 + 环境变量覆盖（Phase 6+）

## 下一步计划

- Phase 9: 流式输出优化
- Phase 10: 多轮对话记忆优化
- Phase 11: Tool calling 的错误恢复
- Phase 12: UI 美化与交互优化

---

**最后更新**: 2026-05-21 (Phase 8 完成)
