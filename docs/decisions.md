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
