# mini-buddy 渐进式实现计划

## 设计原则

- **每个阶段结束时都是一个"可运行"的程序**
- 由简及繁，每步只增加一个核心概念
- 前一阶段是后一阶段的基础，不需要推翻重写
- 先跑通再优化，先丑陋再优雅

---

## Phase 0: 项目骨架 (Day 1)

**目标：** cargo 项目初始化，能编译运行

```
agent-test/
├── Cargo.toml
├── src/
│   └── main.rs
├── docs/
│   └── idea.md
└── .gitignore
```

**做什么：**
- `cargo init`
- 添加基础依赖（tokio, reqwest, serde, serde_json）
- main.rs 打印 "Hello, mini-buddy!"
- git init + 首次提交

**验证：** `cargo run` 正常输出

---

## Phase 1: 最简 LLM 调用 (Day 1-2)

**目标：** 能在终端里和大模型对话一轮（单轮问答，非流式）

**模块：**
```
src/
├── main.rs          # 入口，读取用户输入 → 调用 LLM → 打印回复
└── llm/
    ├── mod.rs
    └── openai.rs    # OpenAI 兼容接口（先做这个，覆盖面最广）
```

**做什么：**
- 定义 `LlmProvider` trait：`async fn chat(messages) -> Result<String>`
- 实现 OpenAI 兼容的 HTTP 调用（reqwest + serde）
- 用 `std::io::stdin` 做最简单的输入
- 硬编码 API key（或从环境变量读取）
- 支持 DeepSeek / Qwen / Kimi（它们都是 OpenAI 兼容格式）

**验证：** 运行后输入问题，能收到模型的回复

---

## Phase 2: 多轮对话 + 流式输出 (Day 2-3)

**目标：** 支持连续对话，回复逐字打出

**改动：**
- 维护 `Vec<Message>` 对话历史
- 实现 SSE streaming 解析（`reqwest` + 逐行读取 `data:` 字段）
- 逐 token 打印到 stdout（`print!` + `flush`）
- 添加 Anthropic API provider（Messages API 格式不同）

**新增模块：**
```
src/llm/
├── anthropic.rs     # Anthropic Messages API
└── types.rs         # Message, Role, StreamChunk 等公共类型
```

**验证：** 连续对话 3 轮以上，回复流式输出无乱码

---

## Phase 3: Agent Loop + Tool Use (Day 3-5)

**目标：** 模型能自主决定调用工具，实现"思考→行动→观察"循环

**这是项目的核心转折点 — 从"聊天机器人"变成"Agent"**

**改动：**
- 定义 `Tool` trait：`name()`, `description()`, `parameters_schema()`, `execute()`
- 实现两个内置工具：
  - `bash`: 执行终端命令，返回 stdout/stderr
  - `read_file`: 读取文件内容
- 实现 Agent Loop：
  ```
  loop {
      response = llm.chat(messages)  // 带 tools 定义
      if response.has_tool_calls:
          for tool_call in response.tool_calls:
              result = tool_registry.execute(tool_call)
              messages.push(tool_result)
      else:
          print(response.text)
          break
  }
  ```
- 在 system prompt 中注入工具描述

**新增模块：**
```
src/
├── agent/
│   ├── mod.rs       # Agent Loop 主逻辑
│   └── prompt.rs    # System prompt 构建
└── tools/
    ├── mod.rs       # Tool trait + ToolRegistry
    ├── bash.rs      # 命令执行
    └── read_file.rs # 文件读取
```

**验证：** 
- 问"当前目录有什么文件？" → Agent 自动调用 bash `ls` 
- 问"读一下 Cargo.toml" → Agent 自动调用 read_file

---

## Phase 4: Bash 沙盒 + 权限控制 (Day 5-6)

**目标：** 危险命令拦截，写操作需确认

**改动：**
- Bash 工具增加命令预检：
  - 黑名单匹配（`rm -rf`, `mkfs`, `dd`, `:(){ :|:& };:`）
  - 正则规则引擎
- 权限分级：
  - 读操作（ls, cat, find, grep）→ 自动执行
  - 写操作（touch, mkdir, cp）→ 提示确认
  - 危险操作（rm, chmod 777）→ 拦截 + 警告
- 添加 `write_file` 工具（带确认流程）

**新增模块：**
```
src/tools/
├── sandbox.rs       # 命令安全检查
└── write_file.rs    # 文件写入（需确认）
```

**验证：** 
- 让 Agent 执行 `rm -rf /` → 被拦截
- 让 Agent 创建文件 → 弹出确认提示

---

## Phase 5: TUI 基础界面 (Day 6-9)

**目标：** 从裸 stdin/stdout 升级到 Ratatui 界面

**改动：**
- 引入 Ratatui + crossterm
- 基础布局：
  ```
  ┌─────────────────────────────┐
  │     对话历史区（可滚动）      │
  │                             │
  │                             │
  ├─────────────────────────────┤
  │     输入区（多行编辑）        │
  └─────────────────────────────┘
  ```
- 异步架构重构：
  - UI 线程：渲染 + 输入事件
  - Agent 线程：LLM 调用 + 工具执行
  - 通过 `tokio::sync::mpsc` channel 通信
- 流式输出逐字渲染到 TUI
- 支持 Ctrl+C 退出、上下滚动

**新增模块：**
```
src/
├── tui/
│   ├── mod.rs       # App 状态 + 事件循环
│   ├── ui.rs        # 布局渲染
│   ├── input.rs     # 输入处理（多行编辑器）
│   └── event.rs     # 事件定义（Key, Resize, AgentMsg）
```

**关键设计决策：**
- 采用 Elm 架构（Event → Update → View）
- panic hook 恢复终端状态

**验证：** 完整 TUI 界面中和 Agent 对话，工具调用过程可见

---

## Phase 6: 配置系统 + 多模型切换 (Day 9-10)

**目标：** 不再硬编码，用配置文件管理一切

**改动：**
- 配置文件 `~/.mini-buddy/config.toml`：
  ```toml
  [default]
  provider = "deepseek"
  
  [providers.deepseek]
  api_key_env = "DEEPSEEK_API_KEY"
  base_url = "https://api.deepseek.com/v1"
  model = "deepseek-chat"
  
  [providers.anthropic]
  api_key_env = "ANTHROPIC_API_KEY"
  model = "claude-sonnet-4-20250514"
  
  [safety]
  blocked_commands = ["rm -rf /", "mkfs"]
  auto_approve_read = true
  ```
- TUI 中支持 `/model <name>` 命令切换模型
- 启动时加载配置，缺失则生成默认模板

**新增模块：**
```
src/
├── config/
│   ├── mod.rs       # Config struct + 加载逻辑
│   └── default.toml # 默认配置模板
```

**验证：** 修改配置文件切换模型，无需重新编译

---

## Phase 7: 上下文管理 + 会话持久化 (Day 10-12)

**目标：** 对话不丢失，上下文不爆炸

**改动：**
- Token 计数（用 `tiktoken-rs` 或简单字符估算）
- 上下文截断策略：
  - 保留 system prompt + 最近 N 轮
  - 超限时摘要旧消息（可选：调用 LLM 生成摘要）
- 会话持久化：
  - SQLite 存储对话历史
  - 启动时列出历史会话，可恢复
- TUI 中支持 `/save`, `/load`, `/history` 命令

**新增模块：**
```
src/
├── context/
│   ├── mod.rs       # ContextManager
│   ├── token.rs     # Token 计数
│   └── storage.rs   # SQLite 持久化
```

**验证：** 退出后重启，能加载之前的对话继续

---

## Phase 8: MCP 支持 (Day 12-15)

**目标：** 支持 Model Context Protocol，外部工具无缝接入

**改动：**
- 实现 MCP 客户端（JSON-RPC over stdio）
- MCP Server 进程管理（spawn + 通信）
- MCP 工具自动注册到 ToolRegistry
- 配置文件中声明 MCP servers：
  ```toml
  [mcp.filesystem]
  command = "npx"
  args = ["-y", "@anthropic/mcp-filesystem"]
  ```

**新增模块：**
```
src/
├── mcp/
│   ├── mod.rs       # MCP Client
│   ├── transport.rs # stdio JSON-RPC 传输
│   └── types.rs     # MCP 协议类型定义
```

**验证：** 接入一个外部 MCP server，Agent 能发现并使用其工具

---

## Phase 9: UI 美化 + Markdown 渲染 (Day 15-17)

**目标：** 炫酷的终端体验

**改动：**
- 流式输出逐 token 渲染（需要 `chat_stream_with_tools()` 或混合模式）
- Markdown 渲染（代码块语法高亮、列表、标题、粗体等）
- 代码 Diff 着色展示（绿色添加、红色删除）
- reasoning_content（思考过程）折叠/展开显示
- 工具调用过程动画（spinner、进度条）
- 主题系统（亮色/暗色）
- 多行输入编辑器（替代 Phase 5 的单行输入）
- 上下滚动聊天历史（键盘 PageUp/Down）
- Unicode 宽度计算（中文字符正确占 2 列宽）

**新增模块：**
```
src/tui/
├── markdown.rs      # Markdown → Ratatui Spans 转换
├── diff.rs          # Diff 着色渲染
├── input.rs         # 多行输入编辑器
└── theme.rs         # 颜色主题
```

**新增依赖（可选）：**
```toml
unicode-width = "0.2"      # 中文宽度计算
syntect = "5"              # 语法高亮（可选，也可用简单规则）
```

**验证：** 代码块有语法高亮，Diff 有颜色区分，支持滚动和多行输入

---

## Phase 10: 可观测性 + Debug 模式 (Day 17-18)

**目标：** 学习者能看清 Agent 每一步在做什么

**改动：**
- Debug 面板（可通过 F12 或 `/debug` 切换）：
  ```
  ┌──────────────┬──────────────┐
  │   对话区域    │  Debug 面板   │
  │              │ > Prompt sent │
  │              │ > Tool: bash  │
  │              │ > Result: ... │
  ├──────────────┴──────────────┤
  │         输入区               │
  └─────────────────────────────┘
  ```
- 记录每轮循环的完整 JSON（request/response）
- 支持导出为文件供分析

**验证：** 开启 Debug 模式，能看到完整的 API 请求和工具调用链

---

## Phase 11: 重试机制 + 错误恢复 (Day 18-19)

**目标：** 生产级的健壮性

**改动：**
- 指数退避重试（429 rate limit、5xx、网络超时）
- 工具执行超时控制（Bash 命令限时）
- Agent Loop 最大循环次数限制（防止死循环）
- 优雅的错误展示（TUI 中显示错误类型 + 重试状态）
- Ctrl+C 中断当前 Agent 循环（不退出程序）

**验证：** 断网后重连能自动恢复，死循环能被打断

---

## Phase 12: SKILL 系统 + 彩蛋 (Day 19-21)

**目标：** 可扩展的技能系统

**改动：**
- SKILL 定义格式（Markdown 或 TOML）
- 内置几个示例 SKILL：
  - `/explain` - 解释代码
  - `/refactor` - 重构建议
  - `/test` - 生成测试
- 用户自定义 SKILL 目录 `~/.mini-buddy/skills/`
- 趣味彩蛋（启动 ASCII Art、特殊指令触发动画等）

**验证：** `/explain src/main.rs` 能触发对应 SKILL 流程

---

## 里程碑总览

| Phase | 耗时 | 产出 | 核心学习点 |
|-------|------|------|-----------|
| 0 | 0.5d | 可编译的空项目 | Cargo 项目结构 |
| 1 | 1d | 单轮 CLI 问答 | HTTP API 调用、async/await |
| 2 | 1.5d | 多轮流式对话 | SSE 解析、消息历史 |
| 3 | 2d | **Agent 循环** | Tool Use、Function Calling |
| 4 | 1d | 安全沙盒 | 命令解析、权限模型 |
| 5 | 3d | **TUI 界面** | Ratatui、异步事件循环 |
| 6 | 1.5d | 配置系统 | TOML 解析、多态 Provider |
| 7 | 2d | 上下文管理 | Token 计数、SQLite |
| 8 | 3d | **MCP 支持** | JSON-RPC、进程管理 |
| 9 | 2d | 美化 UI | Markdown 渲染、主题 |
| 10 | 1.5d | Debug 模式 | 可观测性设计 |
| 11 | 1.5d | 错误恢复 | 重试策略、超时控制 |
| 12 | 2d | SKILL 系统 | 插件架构 |

**总计：约 21 个工作日（3 周）**

---

## 推荐 Crate 清单

```toml
[dependencies]
# 异步运行时
tokio = { version = "1", features = ["full"] }

# HTTP 客户端
reqwest = { version = "0.12", features = ["json", "stream"] }

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# TUI
ratatui = "0.29"
crossterm = "0.28"

# 配置
toml = "0.8"
dirs = "5"                    # 获取 home 目录

# 数据库
rusqlite = { version = "0.31", features = ["bundled"] }

# 错误处理
anyhow = "1"
thiserror = "2"

# 工具
uuid = { version = "1", features = ["v4"] }
chrono = "0.4"
unicode-width = "0.2"        # 中文宽度计算

# 流式解析
futures = "0.3"
tokio-stream = "0.1"
```

---

## 开发建议

1. **每完成一个 Phase 就 git tag** — 方便回溯和学习者按阶段阅读
2. **Phase 1-4 是核心** — 先不碰 TUI，用裸终端跑通 Agent 逻辑
3. **Phase 5 是最大重构** — 引入 TUI 后架构会显著变化，提前预留异步通信的接口
4. **测试策略** — 对 LLM 调用 mock，对工具执行写集成测试
5. **写注释** — 教学项目，关键设计决策处写清楚"为什么这样做"
