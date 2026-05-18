# mini-buddy

## 项目概述

mini-buddy 是一个极简架构的轻量级 AI 编程智能体（教学项目），使用 Rust 构建。目标是剖析、学习和复刻大厂 Agent 架构。

## 当前进度

- ✅ Phase 0: 项目骨架（Cargo 初始化，基础依赖配置）
- ✅ Phase 1: 最简 LLM 调用（单轮问答，OpenAI 兼容接口）
- ✅ Phase 2: 多轮对话 + 流式输出
- ✅ Phase 3: Agent Loop + Tool Use
- ⬜ Phase 4: Bash 沙盒 + 权限控制
- ⬜ Phase 5: TUI 基础界面（Ratatui）
- ⬜ Phase 6-12: 配置系统、上下文管理、MCP、UI美化等

完整计划详见 `docs/plan.md`

## 架构分层

```
TUI Layer → Agent Loop → Tool Registry → LLM Provider → Context/State Manager
```

## 技术栈

- **语言**: Rust (edition 2024)
- **异步运行时**: tokio
- **HTTP**: reqwest (with stream feature)
- **序列化**: serde + serde_json
- **错误处理**: anyhow + thiserror
- **TUI** (Phase 5+): ratatui + crossterm

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
```

## 开发约定

- 每完成一个 Phase 打 git tag（如 `v0.1-phase1`）
- 先用裸终端跑通逻辑（Phase 1-4），再引入 TUI（Phase 5）
- LLM Provider 统一用 trait 抽象，方便多模型切换
- 教学项目，关键设计决策处写注释说明"为什么这样做"
- 中文注释和文档

## 构建与运行

```bash
cargo build
cargo run
```

## 环境变量（Phase 1 开始需要）

```bash
export DEEPSEEK_API_KEY="your-key"      # DeepSeek
export ANTHROPIC_API_KEY="your-key"     # Anthropic Claude
export OPENAI_API_KEY="your-key"        # OpenAI 兼容接口
```

## 关键设计参考

- Agent Loop: 感知→思考→行动循环，支持 CoT 和 reasoning_content
- Tool Use: OpenAI function calling / Anthropic tool use 格式
- 安全: Bash 命令黑名单 + 权限分级（读自动/写确认/危险拦截）
- 异步: UI 线程与 Agent 线程通过 mpsc channel 通信（Phase 5+）
