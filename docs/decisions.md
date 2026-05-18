# 关键设计决策记录

> 记录每个 Phase 的核心架构决策及其原因，供学习者理解"为什么这样做"。

---

## Phase 1：最简 LLM 调用

### D1-1：用 `LlmProvider` trait 抽象 Provider

**决策**：定义 `trait LlmProvider { async fn chat(...) -> Result<String> }`，而非直接在 main.rs 里调用 HTTP。

**原因**：
- Phase 3 的 Agent Loop 只需面向这个接口编程，不感知底层是 DeepSeek 还是 Anthropic
- 换模型时，只需换 `Box<dyn LlmProvider>` 的具体实例，上层零改动
- 教学意义：展示 Rust trait 作为"接口"的多态用法

**代价**：引入 `async-trait` crate（Rust 原生 trait 暂不支持 async fn，需要此宏包装）

---

### D1-2：先实现 OpenAI 兼容格式，不先做 Anthropic

**决策**：Phase 1 只实现 OpenAI Chat Completions 格式（`/v1/chat/completions`），Anthropic Messages API 推迟到 Phase 2。

**原因**：
- OpenAI 兼容格式覆盖面最广：DeepSeek / Qwen / Kimi / Groq / Ollama 全部支持
- 一个实现解锁多个模型，性价比最高
- Anthropic 的 Messages API 格式差异（system 字段独立、流式格式不同）放 Phase 2 单独处理，避免 Phase 1 复杂度过高

---

### D1-3：API Key 从环境变量读取，不硬编码

**决策**：`std::env::var("DEEPSEEK_API_KEY")`，找不到时报清晰错误退出。

**原因**：
- 安全第一：密钥绝不能进版本控制
- 十二因素应用原则：配置与代码分离
- Phase 6 引入配置文件系统后，会支持从 `~/.mini-buddy/config.toml` 读取，但环境变量优先级始终最高

---

### D1-4：`reqwest::Client` 在 Provider 内复用

**决策**：`OpenAIProvider` 结构体持有一个 `reqwest::Client` 字段，而非每次 `chat()` 都 `new()`。

**原因**：
- `Client` 内部维护连接池（keep-alive）和 TLS session 缓存
- 每次 new() 会丢失连接池，造成每次请求都重新握手，延迟增加
- Phase 2 流式输出时尤为重要：长连接 SSE 需要稳定的底层 TCP 连接

---

### D1-5：Phase 1 只做单轮，不维护对话历史

**决策**：`messages` 只包含当前用户输入的一条消息，不累积历史。

**原因**：
- Phase 1 的唯一目标是"跑通一次 API 调用"，聚焦最小可验证单元
- `chat()` 接口签名 `&[Message]` 已预留多轮能力，Phase 2 只需在外部维护 `Vec<Message>` 并传入，接口本身不需要改
- 过早引入状态管理会模糊学习重点

---

### D1-6：响应结构体只反序列化需要的字段

**决策**：`ChatResponse` 只包含 `choices`，忽略 `usage`、`id`、`created` 等字段。

**原因**：
- `serde` 默认忽略 JSON 中多余的字段（无需 `#[serde(deny_unknown_fields)]`）
- 保持结构体简洁，降低阅读成本
- Phase 10 可观测性阶段需要 `usage`（token 计数）时再补充

---

## Phase 2：多轮对话 + 流式输出

### D2-1：chat_stream() 返回 Stream 而非 callback

**决策**：`chat_stream()` 返回 `Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + 'a>>`

**原因**：
- Stream 可组合：可以 map / filter / forward 到 channel
- Phase 5 接入 TUI 时，只需 `while let Some(chunk) = stream.next().await { tx.send(chunk) }` 即可通过 mpsc channel 传递 token
- callback 会把消费逻辑耦合进 Provider 内部，不利于架构分层
- Stream 是 Rust 异步编程的惯用模式，教学意义好

**代价**：需要 `futures` crate 的 `Stream` trait 和 `StreamExt`

---

### D2-2：使用 async-stream 而非手动实现 Stream

**决策**：用 `async_stream::stream!` 宏生成 Stream，而不是手动实现 `poll_next()`

**原因**：
- 手动实现 `Stream` 需要编写状态机，代码量大且难读
- `stream!` 宏允许用顺序 async 代码 + `yield` 来生成 Stream，逻辑清晰
- 对教学项目来说，学生能看到"请求 → 解析 → yield token"的线性流程
- 编译器会帮我们生成高效的状态机，性能不受影响

**代价**：引入 `async-stream` crate

---

### D2-3：手动解析 SSE 而非使用 eventsource 库

**决策**：手动按行解析 `data:` 前缀和 `\n\n` 分隔符，不使用 `eventsource-stream` 等库

**原因**：
- SSE 协议极简：数据行以 `data: ` 为前缀，事件以双换行分隔
- 手动解析让学习者理解 SSE 协议的本质，不被库的抽象遮蔽
- 只需处理 `data:` 行，忽略 `event:`、`id:`、`retry:` 等（Anthropic 除外，需要 `event:` 区分事件类型）
- 代码量少于 50 行，不值得引入额外依赖

---

### D2-4：类型定义移至 types.rs

**决策**：将 `Role`、`Message` 从 `mod.rs` 移至 `types.rs`，新增 `StreamChunk`

**原因**：
- Phase 2 有两个 Provider（openai.rs + anthropic.rs）都需要这些类型
- 类型是"数据"，trait 是"契约"，分离关注点
- `mod.rs` 只保留 trait 定义和模块声明，保持简洁
- 新增 `StreamChunk` 枚举区分 Delta（token 片段）和 Done（结束信号），为 Phase 5 TUI 预留状态区分能力

---

### D2-5：Provider 选择通过环境变量

**决策**：用 `LLM_PROVIDER` 环境变量选择 Provider（默认 deepseek），工厂函数 `create_provider()` 返回 `Box<dyn LlmProvider>`

**原因**：
- 简单，不需要引入 clap 等参数解析库
- 环境变量方便在 shell 中快速切换：`LLM_PROVIDER=anthropic cargo run`
- 工厂函数集中注册逻辑，新增 Provider 只改一处
- Phase 6 配置系统引入后会改为从配置文件读取，但环境变量优先级始终最高

---

### D2-6：用作用域块解决借用冲突

**决策**：在 `main.rs` 中用 `{ let stream = ...; ... }` 块限定 stream 的生命周期

**原因**：
- `chat_stream(&history)` 不可变借用了 `history`
- 循环结尾的 `history.push()` 需要可变借用
- Rust 借用检查器不允许同时存在不可变和可变借用
- 用作用域块确保 stream 在 push 前被 drop，释放不可变借用
- 这是 Rust 中处理此类问题的惯用模式

---

## Phase 3：Agent Loop + Tool Use

### D3-1：新增 `chat_with_tools()` 而非修改已有 `chat()` 签名

**决策**：在 `LlmProvider` trait 中新增 `chat_with_tools(&self, messages, tools) -> Result<LlmResponse>` 方法，保留原有 `chat()` 和 `chat_stream()` 不变。

**原因**：
- 向后兼容：Phase 2 的 `chat()` / `chat_stream()` 调用者不受影响
- 返回类型不同：`chat()` 返回 `String`，`chat_with_tools()` 返回包含工具调用的 `LlmResponse`
- 关注点分离：普通对话不需要工具调用的复杂逻辑
- Phase 5 TUI 可能仍用 `chat_stream()` 做纯对话场景的流式输出

**代价**：trait 方法增多，每个 Provider 需实现更多方法

---

### D3-2：Agent Loop 使用非流式调用

**决策**：Agent Loop 内部使用 `chat_with_tools()`（非流式），不使用流式传输。

**原因**：
- Agent Loop 需要**完整响应**才能判断是否包含工具调用
- 流式传输下，工具调用信息分散在多个 chunk 中，必须全部收集后才能执行工具
- 非流式简化了循环逻辑：调用 → 判断 → 执行 → 循环
- Phase 5 的 TUI 引入后，可以在最终回答阶段切回流式输出（混合模式）

**代价**：Agent 思考时用户看不到逐字输出，有等待感。Phase 5 解决。

---

### D3-3：Message 扩展 Option 字段而非新建类型

**决策**：在现有 `Message` 结构体上新增 `tool_calls: Option<Vec<ToolCall>>`、`tool_call_id: Option<String>`、`name: Option<String>` 三个可选字段。

**原因**：
- 对话历史是一个统一的 `Vec<Message>`，需要混合存储 user/assistant/tool 三种消息
- 用 Option 字段而非 enum 变体，是因为 API 格式本身就是"同一个 message 结构 + 可选字段"
- 旧的构造器（`Message::user()`、`Message::assistant()`）中新字段均为 None，100% 向后兼容
- 移除了 `#[derive(Serialize)]`，因为两个 Provider 都手动构建 JSON（不同 API 格式差异太大）

**替代方案**：用 `enum MessageContent { Text(String), ToolCalls(Vec<ToolCall>), ToolResult{...} }` 做强类型区分。但会让 Provider 的消息转换代码更复杂，教学清晰度下降。

---

### D3-4：工具执行错误返回文本而非 Err

**决策**：工具执行失败时返回错误描述字符串（`Ok("工具执行出错: ...")`），而非 `Err()`。

**原因**：
- LLM 需要看到错误信息来调整策略（如文件不存在时尝试其他路径）
- 如果返回 Err，Agent Loop 就会终止整个交互，用户得重新提问
- 错误信息也是对 LLM 有价值的"观察"——这是 Agent 的核心理念
- 只有"系统级错误"（如网络故障）才应该用 Err 终止循环

---

### D3-5：ToolRegistry 使用 HashMap 而非 Vec

**决策**：`ToolRegistry` 内部用 `HashMap<String, Box<dyn Tool>>` 存储工具。

**原因**：
- Agent Loop 每轮可能执行多次工具调用，按名字查找是 O(1)
- 保证工具名唯一性（HashMap key 天然去重）
- Phase 8 MCP 动态注册工具时，HashMap 的增删查改都是 O(1)

---

### D3-6：Anthropic 连续 tool_result 消息合并

**决策**：`build_tool_request_body()` 中检测连续的 `Role::Tool` 消息，将它们合并为单个 `user` 消息的多个 content blocks。

**原因**：
- Anthropic API 不允许连续的同角色消息（会返回 400 错误）
- 当 LLM 一次返回多个 tool_calls 时，每个工具结果都是 `Role::Tool`，转换后都变成 `role: "user"`
- 必须合并到一个 user 消息中，用 `content: [{type: "tool_result"}, {type: "tool_result"}]` 格式
- OpenAI 无此限制（每个 tool result 独立一条 `role: "tool"` 消息即可）

---

### D3-7：MAX_ITERATIONS = 10 作为安全阀

**决策**：Agent Loop 最多循环 10 次，超过后返回兜底消息。

**原因**：
- 防止 LLM 陷入无限工具调用（如反复查同一个文件）
- 大多数真实任务 2-3 轮即可完成（查找 → 读取 → 回答）
- 10 轮足够处理复杂多步任务，同时是安全的上限
- Phase 11 将引入更细粒度的循环控制（用户可 Ctrl+C 中断）

---

### D3-8：Bash 工具用 `sh -c` 而非直接执行

**决策**：`BashTool` 使用 `Command::new("sh").arg("-c").arg(command)` 执行命令。

**原因**：
- LLM 生成的是完整 shell 命令字符串（如 `ls -la | grep .rs`）
- 管道符 `|`、重定向 `>`、命令链接 `&&` 都是 shell 语法，`Command::new("ls")` 无法解释
- `sh -c` 让 shell 来解析整个命令字符串，支持所有 shell 特性
- Phase 4 将在 `sh -c` 前加安全预检层（命令黑名单匹配）
