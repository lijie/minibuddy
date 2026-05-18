/// TUI 事件类型定义
///
/// 定义 Agent 任务和 UI 任务之间的通信协议。
/// 两个方向：
/// - Agent → UI：AgentEvent（状态更新、工具调用、最终回答、确认请求）
/// - UI → Agent：UserAction（用户输入、退出）
///
/// 确认流程使用嵌入的 oneshot channel 实现双向通信。

use tokio::sync::oneshot;

// ────────────────────────────────────────────────────────────
// Agent → UI 事件
// ────────────────────────────────────────────────────────────

/// Agent 任务发送给 UI 的事件
///
/// UI 收到事件后更新 App 状态并触发重绘。
/// 设计原则：每个事件都是自包含的，UI 不需要额外查询 Agent 状态。
pub enum AgentEvent {
    /// Agent 开始新一轮思考
    ThinkingStarted { iteration: u32 },
    /// LLM 返回了思考内容（调用工具前的文本）
    ThinkingContent(String),
    /// 开始执行工具调用
    ToolCallStart { name: String, args: String },
    /// 工具执行完成，返回结果
    ToolCallResult(String),
    /// Agent 给出最终回答（完整文本，Phase 5 不做流式）
    FinalResponse(String),
    /// Agent 执行出错
    Error(String),
    /// Agent 请求用户确认写操作
    /// 内嵌 oneshot::Sender 用于接收 UI 的确认结果
    ConfirmationRequest {
        message: String,
        response_tx: oneshot::Sender<bool>,
    },
}

// ────────────────────────────────────────────────────────────
// UI → Agent 动作
// ────────────────────────────────────────────────────────────

/// 用户在 UI 中触发的动作
pub enum UserAction {
    /// 用户提交了一条输入
    Submit(String),
    /// 用户请求退出
    Quit,
}

// ────────────────────────────────────────────────────────────
// UI 状态类型
// ────────────────────────────────────────────────────────────

/// 聊天消息的角色（决定渲染样式）
#[derive(Clone)]
pub enum ChatRole {
    /// 用户输入（青色）
    User,
    /// 助手最终回答（白色）
    Assistant,
    /// 工具调用信息（黄色）
    ToolCall,
    /// 工具执行结果（灰色）
    ToolResult,
    /// LLM 思考内容（品红色）
    Thinking,
    /// 状态信息如"第 N 轮思考中"（蓝色）
    Status,
    /// 错误信息（红色）
    Error,
}

/// 一条聊天记录
#[derive(Clone)]
pub struct ChatEntry {
    pub role: ChatRole,
    pub content: String,
}

/// 输入框的模式
#[derive(Clone, Copy, PartialEq)]
pub enum InputMode {
    /// 正常输入模式（可打字、按 Enter 发送）
    Normal,
    /// 等待 Agent 响应（输入禁用）
    WaitingForAgent,
    /// 确认对话框（只响应 y/n）
    ConfirmationDialog,
}
