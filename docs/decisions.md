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
