/// Agent 模块：实现"思考→行动→观察"循环
///
/// 这是整个项目的核心——让 LLM 从"聊天机器人"进化为"自主代理"。
///
/// Phase 5 变更：
/// - Agent 通过 event_tx channel 向 TUI 发送状态事件（替代 println!）
/// - 用户确认通过 oneshot channel 异步完成（替代阻塞 stdin）
/// - run() 返回 Result<()>（最终回答通过 AgentEvent::FinalResponse 发送）
///
/// 架构：Agent 在独立 tokio task 中运行，与 TUI task 通过 mpsc 通信

pub mod prompt;

use anyhow::Result;
use serde_json::Value;
use std::io::Write;
use tokio::sync::{mpsc, oneshot};

use crate::llm::{LlmProvider, LlmResponse, Message, ToolCall};
use crate::tools::sandbox::{self, PermissionLevel};
use crate::tools::ToolRegistry;
use crate::tui::event::AgentEvent;
use crate::context::ContextManager;

/// 最大工具调用轮次（安全阀，防止无限循环）
const MAX_ITERATIONS: usize = 10;

/// 日志文件路径
const LOG_FILE: &str = "mini-buddy.log";

/// 写入调试日志到文件（不影响 TUI 显示）
///
/// TUI 接管了终端，stderr 输出会破坏界面。
/// 文件日志可以用 `tail -f mini-buddy.log` 实时查看。
fn log(msg: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
    {
        let _ = writeln!(file, "[{}] {}", timestamp(), msg);
    }
}

/// 公开的日志接口（供 main.rs 等外部模块使用）
pub fn log_info(msg: &str) {
    log(msg);
}

/// 写入多行日志（带缩进，不加时间戳前缀）
fn log_block(header: &str, body: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
    {
        let _ = writeln!(file, "[{}] {}", timestamp(), header);
        for line in body.lines() {
            let _ = writeln!(file, "  │ {}", line);
        }
        let _ = writeln!(file, "  └─");
    }
}

/// 记录完整的消息列表（发送给 LLM 前的对话历史）
fn log_messages(messages: &[Message]) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
    {
        let _ = writeln!(file, "[{}] ─── 发送消息列表 ({} 条) ───", timestamp(), messages.len());
        for (i, msg) in messages.iter().enumerate() {
            let role_tag = match msg.role {
                crate::llm::Role::System => "SYSTEM",
                crate::llm::Role::User => "USER",
                crate::llm::Role::Assistant => "ASSISTANT",
                crate::llm::Role::Tool => "TOOL",
            };

            // 内容预览（截断到 300 字符）
            let content_preview: String = msg.content.chars().take(300).collect();
            let suffix = if msg.content.chars().count() > 300 { "..." } else { "" };

            let _ = writeln!(file, "  [{}] #{} {}", role_tag, i, content_preview);
            if !suffix.is_empty() {
                let _ = writeln!(file, "       (共 {} 字符，已截断)", msg.content.chars().count());
            }

            // 如果有 tool_calls，打印出来
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    let _ = writeln!(file, "       ↳ tool_call: {}({}) id={}", tc.name, tc.arguments, tc.id);
                }
            }

            // 如果是 tool 结果，显示关联的 tool_call_id
            if let Some(ref id) = msg.tool_call_id {
                let _ = writeln!(file, "       ↳ tool_call_id: {}", id);
            }
        }
        let _ = writeln!(file, "  ─── 消息列表结束 ───");
    }
}

/// 记录 LLM 响应
fn log_response(response: &LlmResponse) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_FILE)
    {
        let _ = writeln!(file, "[{}] ─── LLM 响应 ───", timestamp());

        // 文本内容
        match &response.content {
            Some(text) if !text.is_empty() => {
                let preview: String = text.chars().take(500).collect();
                let suffix = if text.chars().count() > 500 { "..." } else { "" };
                let _ = writeln!(file, "  content ({} 字符):", text.chars().count());
                for line in preview.lines() {
                    let _ = writeln!(file, "  │ {}", line);
                }
                if !suffix.is_empty() {
                    let _ = writeln!(file, "  │ ...(已截断)");
                }
            }
            _ => {
                let _ = writeln!(file, "  content: (空)");
            }
        }

        // 工具调用
        if response.tool_calls.is_empty() {
            let _ = writeln!(file, "  tool_calls: (无)");
        } else {
            let _ = writeln!(file, "  tool_calls ({} 个):", response.tool_calls.len());
            for tc in &response.tool_calls {
                let _ = writeln!(file, "  ├─ [{}] {}({})", tc.id, tc.name, tc.arguments);
            }
        }

        let _ = writeln!(file, "  ─── 响应结束 ───");
    }
}

/// 简易时间戳
fn timestamp() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, mins, s)
}

// ────────────────────────────────────────────────────────────
// 权限检查相关类型
// ────────────────────────────────────────────────────────────

/// 权限检查后应采取的行动
enum PermissionAction {
    AutoExecute,
    NeedConfirmation { prompt_message: String },
    Blocked { reason: String },
}

// ────────────────────────────────────────────────────────────
// Agent 结构体
// ────────────────────────────────────────────────────────────

/// Agent：将 LLM Provider 和工具系统组合在一起的核心结构
///
/// Phase 5 新增 event_tx：通过 channel 向 TUI 发送状态事件，
/// 替代了之前直接 println! 的方式。这样 Agent 可以在独立 task 中运行，
/// 不会阻塞 UI 渲染。
pub struct Agent {
    provider: Box<dyn LlmProvider>,
    tool_registry: ToolRegistry,
    messages: Vec<Message>,
    /// 向 TUI 发送事件的 channel
    event_tx: mpsc::Sender<AgentEvent>,
    /// 上下文管理器（Phase 7）：token 估算 + 截断
    context_manager: ContextManager,
}

impl Agent {
    /// 创建新的 Agent 实例
    pub fn new(
        provider: Box<dyn LlmProvider>,
        tool_registry: ToolRegistry,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Self {
        let tool_defs = tool_registry.definitions();
        let system_prompt = prompt::build_system_prompt(&tool_defs);

        Self {
            provider,
            tool_registry,
            messages: vec![Message::system(system_prompt)],
            event_tx,
            context_manager: ContextManager::default(),
        }
    }

    /// 获取消息历史（用于会话保存）
    pub fn get_messages(&self) -> &[Message] {
        &self.messages
    }

    /// 设置消息历史（用于会话加载）
    pub fn set_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// 运行一次完整的 Agent 交互
    ///
    /// Phase 5 变更：不再返回最终文本（通过 FinalResponse 事件发送），
    /// 返回 Result<()> 表示执行是否成功。
    pub async fn run(&mut self, user_input: &str) -> Result<()> {
        // ── Step 1: 将用户消息加入对话历史 ──
        self.messages.push(Message::user(user_input));
        log(&format!("════════ 新对话轮次 ════════"));
        log(&format!("USER: {}", user_input));

        let tool_defs = self.tool_registry.definitions();

        // ── Step 2: Agent Loop ──
        let mut iterations = 0;

        loop {
            iterations += 1;

            if iterations > MAX_ITERATIONS {
                let fallback =
                    "抱歉，我尝试了太多次工具调用但未能完成任务。请尝试简化你的问题。".to_string();
                self.messages.push(Message::assistant(&fallback));
                let _ = self.event_tx.send(AgentEvent::FinalResponse(fallback)).await;
                return Ok(());
            }

            // ── 思考：通知 TUI ──
            log(&format!("── 第 {} 轮迭代 ──", iterations));

            // Phase 7：上下文截断——确保消息不超过 token 限制
            let removed = self.context_manager.truncate_if_needed(&mut self.messages);
            if removed > 0 {
                log(&format!("⚠ 上下文截断：移除了 {} 条旧消息", removed));
            }

            log_messages(&self.messages);

            let _ = self
                .event_tx
                .send(AgentEvent::ThinkingStarted {
                    iteration: iterations as u32,
                })
                .await;

            let response: LlmResponse = self
                .provider
                .chat_with_tools(&self.messages, &tool_defs)
                .await?;

            log_response(&response);

            // ── 判断：LLM 是否想调用工具？──
            if !response.has_tool_calls() {
                let final_text = response.content.unwrap_or_default();
                log(&format!("✓ 最终回答 ({} 字符)", final_text.chars().count()));
                self.messages.push(Message::assistant(&final_text));
                let _ = self
                    .event_tx
                    .send(AgentEvent::FinalResponse(final_text))
                    .await;
                return Ok(());
            }

            // ── 行动 + 观察 ──

            self.messages.push(Message::assistant_with_tool_calls(
                response.content.clone(),
                response.tool_calls.clone(),
            ));

            // 如果 LLM 有思考文本，通知 TUI
            if let Some(ref thinking) = response.content {
                if !thinking.is_empty() {
                    let _ = self
                        .event_tx
                        .send(AgentEvent::ThinkingContent(thinking.clone()))
                        .await;
                }
            }

            // 依次执行每个工具调用
            for tool_call in &response.tool_calls {
                log(&format!("⚙ 执行工具: {}({})", tool_call.name, tool_call.arguments));

                // 通知 TUI：工具调用开始
                let _ = self
                    .event_tx
                    .send(AgentEvent::ToolCallStart {
                        name: tool_call.name.clone(),
                        args: tool_call.arguments.to_string(),
                    })
                    .await;

                let result = self.execute_tool(tool_call).await;

                log_block(
                    &format!("⚙ 工具结果 [{}]:", tool_call.name),
                    &result,
                );

                // 通知 TUI：工具结果
                let _ = self
                    .event_tx
                    .send(AgentEvent::ToolCallResult(result.clone()))
                    .await;

                // 将工具结果加入对话历史
                self.messages.push(Message::tool_result(
                    &tool_call.id,
                    &tool_call.name,
                    result,
                ));
            }
        }
    }

    // ────────────────────────────────────────────────────────
    // 权限检查 + 确认流程
    // ────────────────────────────────────────────────────────

    /// 执行单个工具调用（含权限检查和确认流程）
    async fn execute_tool(&self, tool_call: &ToolCall) -> String {
        let tool = match self.tool_registry.get(&tool_call.name) {
            Some(t) => t,
            None => {
                let available = self.tool_registry.list_names();
                return format!(
                    "错误：工具 '{}' 不存在。可用工具: {:?}",
                    tool_call.name, available
                );
            }
        };

        // 权限检查
        let action = self.check_permission(&tool_call.name, &tool_call.arguments);

        match action {
            PermissionAction::AutoExecute => {}
            PermissionAction::NeedConfirmation { prompt_message } => {
                // Phase 5：通过 oneshot channel 请求 TUI 确认
                let approved = self.ask_user_confirmation(&prompt_message).await;
                if !approved {
                    return "操作已被用户取消。".to_string();
                }
            }
            PermissionAction::Blocked { reason } => {
                return format!(
                    "⛔ 操作被安全策略阻止：{}\n该命令不会被执行。",
                    reason
                );
            }
        }

        match tool.execute(tool_call.arguments.clone()).await {
            Ok(result) => result,
            Err(e) => format!("工具执行出错: {}", e),
        }
    }

    /// 检查工具调用的权限等级
    fn check_permission(&self, tool_name: &str, arguments: &Value) -> PermissionAction {
        match tool_name {
            "bash" => {
                let command = arguments["command"].as_str().unwrap_or("");
                let result = sandbox::classify(command);

                match result.level {
                    PermissionLevel::Read => PermissionAction::AutoExecute,
                    PermissionLevel::Write => PermissionAction::NeedConfirmation {
                        prompt_message: format!(
                            "$ {}\n分类: {}",
                            command, result.reason
                        ),
                    },
                    PermissionLevel::Dangerous => PermissionAction::Blocked {
                        reason: result.reason,
                    },
                }
            }
            "write_file" => {
                let path = arguments["path"].as_str().unwrap_or("(未知路径)");
                let content = arguments["content"].as_str().unwrap_or("");
                let preview = content_preview(content, 3);

                PermissionAction::NeedConfirmation {
                    prompt_message: format!("写入文件: {}\n{}", path, preview),
                }
            }
            _ => PermissionAction::AutoExecute,
        }
    }

    /// 向 TUI 发送确认请求并等待用户响应
    ///
    /// Phase 5 实现：通过 AgentEvent::ConfirmationRequest 发送确认请求，
    /// 内嵌 oneshot::Sender 用于接收 TUI 的 y/n 结果。
    /// Agent task 在此处 await，直到用户按键。
    async fn ask_user_confirmation(&self, message: &str) -> bool {
        let (response_tx, response_rx) = oneshot::channel();

        let _ = self
            .event_tx
            .send(AgentEvent::ConfirmationRequest {
                message: message.to_string(),
                response_tx,
            })
            .await;

        // 等待 UI 返回确认结果
        // 如果 UI 关闭（channel dropped），默认拒绝
        response_rx.await.unwrap_or(false)
    }

    /// 获取对话历史的引用
    #[allow(dead_code)]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

// ────────────────────────────────────────────────────────────
// 辅助函数
// ────────────────────────────────────────────────────────────

/// 生成文件内容预览（前 N 行 + 省略提示）
fn content_preview(content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let preview_lines: Vec<String> = lines
        .iter()
        .take(max_lines)
        .map(|line| format!("│ {}", line))
        .collect();

    let mut preview = preview_lines.join("\n");

    if total > max_lines {
        preview.push_str(&format!("\n│ ... (共 {} 行)", total));
    }

    preview
}
