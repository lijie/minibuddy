/// LLM Provider 抽象层
///
/// 设计原因：用 trait 隔离具体 Provider 实现（OpenAI / Anthropic / Ollama 等），
/// 上层 Agent Loop 只依赖这个接口，换模型时不需要改动业务逻辑。
///
/// Phase 2 变更：
/// - 类型定义移至 types.rs，这里通过 pub use 重新导出
/// - 新增 chat_stream() 方法支持流式输出
/// - 新增 anthropic 模块
///
/// Phase 3 变更：
/// - 新增 chat_with_tools() 方法，支持带工具定义的对话
/// - 返回 LlmResponse（包含文本 + 工具调用）

pub mod types;
pub mod openai;
pub mod anthropic;

// 重新导出类型，外部使用 `use crate::llm::Message` 即可
pub use types::*;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

/// LLM Provider 统一接口
///
/// Phase 1：只有 chat()
/// Phase 2：新增 chat_stream()，返回异步 token 流
/// Phase 3：新增 chat_with_tools()，支持工具调用
///
/// 为什么用 trait object（Box<dyn LlmProvider>）而不是泛型？
/// - 运行时根据环境变量选择 provider，需要动态分发
/// - Phase 5 的 TUI 持有 Box<dyn LlmProvider>，不关心具体类型
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 非流式调用 — 输入消息列表，输出完整回复文本
    ///
    /// 保留此方法的原因：测试方便 + 向后兼容 + 某些场景不需要流式
    async fn chat(&self, messages: &[Message]) -> Result<String>;

    /// 流式调用 — 返回 token 流
    ///
    /// 为什么返回 Stream 而不是 callback？
    /// - Stream 可组合：可以 map / filter / forward 到 channel
    /// - Phase 5 接入 TUI 时只需 `while let Some(chunk) = stream.next().await { tx.send(chunk) }`
    /// - callback 会把消费逻辑耦合进 Provider 内部，不利于架构分层
    ///
    /// 生命周期 'a 的含义：Stream 的存活时间不超过 &self 和 &messages
    /// 调用方必须保证 messages 在 Stream 被完全消费前不被 drop
    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + 'a>>;

    /// Phase 3: 带工具定义的对话（非流式）
    ///
    /// 为什么非流式？Agent Loop 需要完整响应才能判断是否包含工具调用。
    /// 流式传输下无法预知响应结构（工具调用信息分散在多个 chunk 中），
    /// 必须等待完整结果才能决定下一步动作。
    ///
    /// 为什么加新方法而不是修改 chat() 的签名？
    /// 1. 向后兼容：现有的 chat() 调用者不受影响
    /// 2. 返回类型不同：chat() 返回 String，这里返回更丰富的 LlmResponse
    /// 3. 关注点分离：普通对话不需要处理工具调用的复杂逻辑
    async fn chat_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse>;
}
