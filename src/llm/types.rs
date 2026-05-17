/// 公共类型定义
///
/// 设计原因：将 Message、Role、StreamChunk 等类型集中在此模块，
/// 供所有 Provider（OpenAI / Anthropic）共享，避免循环依赖。
/// Phase 1 这些类型在 mod.rs 中，Phase 2 因为新增 StreamChunk
/// 且 provider 数量增加，拆分到独立文件更清晰。

use serde::Serialize;

// ────────────────────────────────────────────────────────────
// 消息角色
// ────────────────────────────────────────────────────────────

/// 消息角色
///
/// OpenAI 格式：user / assistant / system 都放在 messages 数组
/// Anthropic 格式：system 是独立顶层字段，不在 messages 中（由 Provider 内部处理）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Role {
    /// 序列化为 API 使用的字符串（手动版本，供不使用 serde 的场景）
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        }
    }
}

// ────────────────────────────────────────────────────────────
// 对话消息
// ────────────────────────────────────────────────────────────

/// 单条对话消息
///
/// 这是 mini-buddy 内部的通用消息格式，与具体 API 无关。
/// 各 Provider 负责将其转换为自己的 API 请求格式。
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
}

// ────────────────────────────────────────────────────────────
// 流式输出片段
// ────────────────────────────────────────────────────────────

/// 流式输出的单个片段
///
/// 为什么用 enum 而不是纯 String？
/// - Phase 5 的 TUI 需要区分"内容 token"和"结束信号"来更新 UI 状态
/// - 预留扩展：Phase 3 可增加 ToolCallDelta 变体用于工具调用流式输出
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// 一个文本 token 片段（可能是一个字、一个词、甚至半个字符）
    Delta(String),
    /// 流结束标记
    Done,
}
