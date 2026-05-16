/// LLM Provider 抽象层
///
/// 设计原因：用 trait 隔离具体 Provider 实现（OpenAI / Anthropic / Ollama 等），
/// 上层 Agent Loop 只依赖这个接口，换模型时不需要改动业务逻辑。

pub mod openai;

use anyhow::Result;
use async_trait::async_trait;

/// 消息角色
///
/// OpenAI 格式：user / assistant / system
/// Anthropic 格式类似，但 system 是单独字段（Phase 2 处理）
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Role {
    /// 序列化为 API 使用的字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        }
    }
}

/// 单条对话消息
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
}

/// LLM Provider 统一接口
///
/// Phase 1：只需 chat()，输入消息列表，输出回复文本。
/// Phase 2 会扩展 chat_stream() 支持流式输出。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, messages: &[Message]) -> Result<String>;
}
