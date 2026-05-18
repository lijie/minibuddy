/// 公共类型定义
///
/// 设计原因：将 Message、Role、StreamChunk 等类型集中在此模块，
/// 供所有 Provider（OpenAI / Anthropic）共享，避免循环依赖。
/// Phase 1 这些类型在 mod.rs 中，Phase 2 因为新增 StreamChunk
/// 且 provider 数量增加，拆分到独立文件更清晰。
///
/// Phase 3 变更：
/// - 新增 ToolDefinition、ToolCall、LlmResponse 三个核心类型
/// - Role 新增 Tool 变体
/// - Message 新增 tool_calls、tool_call_id、name 字段
/// - 移除 Message 的 #[derive(Serialize)]（两个 Provider 都是手动构建 JSON）

use serde::Serialize;
use serde_json::Value;

// ────────────────────────────────────────────────────────────
// 消息角色
// ────────────────────────────────────────────────────────────

/// 消息角色
///
/// OpenAI 格式：user / assistant / system / tool 都放在 messages 数组
/// Anthropic 格式：system 是独立顶层字段，tool 结果作为 user 消息的 content block
///
/// Phase 3 新增 Tool 变体：工具执行结果的消息角色
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    /// Phase 3: 工具执行结果
    /// 当 Agent 执行完工具后，用此角色将结果反馈给 LLM
    Tool,
}

impl Role {
    /// 序列化为 API 使用的字符串（手动版本，供不使用 serde 的场景）
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
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
///
/// Phase 3 变更：
/// - 新增 tool_calls / tool_call_id / name 字段，支持工具调用的对话历史
/// - 移除 #[derive(Serialize)]，因为两个 Provider 都手动构建 JSON
///   新增的 Option<Vec<ToolCall>> 字段也无法直接 derive
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,

    // ── Phase 3 新增字段 ──

    /// assistant 消息中携带的工具调用列表
    /// 为什么存在 Message 里？API 要求重放完整对话历史，
    /// 包括之前 assistant 发起的工具调用信息
    pub tool_calls: Option<Vec<ToolCall>>,

    /// tool 角色消息关联的工具调用 ID
    /// OpenAI 格式需要这个字段将工具结果匹配回对应的调用请求
    pub tool_call_id: Option<String>,

    /// 工具名称（部分 API 格式中 tool 结果需要携带）
    pub name: Option<String>,
}

impl Message {
    /// 创建用户消息（Phase 3 新增字段均为 None，向后兼容）
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// 创建助手消息（纯文本，不带工具调用）
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// 创建系统消息
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    // ── Phase 3 新增构造器 ──

    /// 创建带工具调用的 assistant 消息
    ///
    /// 当 LLM 决定调用工具时，需要把这个决定记录到对话历史中，
    /// 后续 API 调用会携带完整历史，包括之前的工具调用
    pub fn assistant_with_tool_calls(
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.unwrap_or_default(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            name: None,
        }
    }

    /// 创建工具执行结果消息
    ///
    /// 将工具输出反馈给 LLM，让它基于结果继续推理。
    /// tool_call_id 用于将结果关联回对应的工具调用请求。
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
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

// ────────────────────────────────────────────────────────────
// Phase 3: 工具相关类型
// ────────────────────────────────────────────────────────────

/// 工具定义：描述一个工具的能力，传给 LLM 让它知道可以调用什么
///
/// 这个结构体是 Tool trait 和 LLM Provider 之间的桥梁：
/// - Tool trait 实现者生成它（通过 name/description/parameters_schema）
/// - LLM Provider 消费它（转换为各自 API 的工具定义格式）
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// 工具的唯一标识名，如 "bash", "read_file"
    pub name: String,
    /// 工具用途的自然语言描述，帮助 LLM 决定何时使用
    pub description: String,
    /// JSON Schema 格式的参数定义
    /// 为什么用 Value 而不是强类型？因为 JSON Schema 本身就是 JSON，
    /// 且不同工具的参数结构各异，Value 最灵活
    pub parameters: Value,
}

/// 工具调用：LLM 决定调用工具时返回的结构
///
/// 一次 LLM 回复可能包含多个工具调用（并行调用），
/// 每个调用有唯一 ID 用于将执行结果关联回对应请求
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// 调用 ID（由 LLM 生成），用于匹配请求和结果
    pub id: String,
    /// 要调用的工具名（对应 ToolDefinition.name）
    pub name: String,
    /// 工具参数（已解析为 JSON Value）
    /// OpenAI 返回 JSON 字符串需解析，Anthropic 直接返回对象
    pub arguments: Value,
}

/// LLM 的完整响应：可能包含文本、工具调用、或两者都有
///
/// 为什么不复用 Message？
/// - Message 是对话历史的单元（存储格式）
/// - LlmResponse 是一次 API 调用的原始返回（传输格式）
/// - 语义不同，分开后各自可以独立演化
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// 文本回复内容（纯工具调用时可能为 None）
    pub content: Option<String>,
    /// 工具调用列表（为空表示 LLM 不想调用工具，直接给出最终回答）
    pub tool_calls: Vec<ToolCall>,
}

impl LlmResponse {
    /// 是否包含工具调用？
    /// 这是 Agent Loop 的核心判断点——决定是继续循环还是返回结果
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}
