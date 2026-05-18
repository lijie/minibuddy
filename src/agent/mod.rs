/// Agent 模块：实现"思考→行动→观察"循环
///
/// 这是整个项目的核心——让 LLM 从"聊天机器人"进化为"自主代理"。
///
/// Agent Loop 工作流程：
/// ```text
/// 用户提问
///   │
///   ▼
/// ┌─────────────────────────────┐
/// │  LLM 思考（chat_with_tools）│ ◄──┐
/// └──────────┬──────────────────┘    │
///            │                       │
///     有工具调用？                    │
///    ╱          ╲                    │
///   是           否                  │
///   │            │                   │
///   ▼            ▼                   │
/// 权限检查    返回最终回答           │
///   │                               │
///   ├─ Read → 自动执行              │
///   ├─ Write → 确认提示 → 执行/取消  │
///   └─ Dangerous → 阻断             │
///   │                               │
///   ▼                               │
/// 将结果加入历史 ───────────────────┘
/// ```
///
/// 关键设计决策：
/// - 使用非流式调用（chat_with_tools）：需要完整响应才能判断是否有工具调用
/// - 最大循环次数限制：防止 LLM 陷入无限工具调用
/// - 错误宽容：工具执行失败不终止循环，而是将错误信息反馈给 LLM
/// - Phase 4：权限检查在 Agent 层，Tool trait 保持纯净无 I/O 交互

pub mod prompt;

use anyhow::Result;
use serde_json::Value;

use crate::llm::{LlmProvider, LlmResponse, Message, ToolCall};
use crate::tools::sandbox::{self, PermissionLevel};
use crate::tools::ToolRegistry;

/// 最大工具调用轮次（安全阀，防止无限循环）
///
/// 大多数任务 2-3 轮就够了（查找→读取→回答），
/// 10 轮是足够宽裕的上限。如果 10 轮还没完成，
/// 很可能是 LLM 陷入了循环或任务本身有问题。
const MAX_ITERATIONS: usize = 10;

// ────────────────────────────────────────────────────────────
// Phase 4: 权限检查相关类型
// ────────────────────────────────────────────────────────────

/// 权限检查后应采取的行动
///
/// 为什么用 enum 而不是 bool？
/// 三种情况（自动/确认/阻断）无法用 bool 表达，
/// 且每种情况携带不同的附加信息（确认提示文本、阻断原因）
enum PermissionAction {
    /// 自动执行（只读操作，或不需要权限检查的工具）
    AutoExecute,
    /// 需要用户确认（写操作）
    NeedConfirmation { prompt_message: String },
    /// 直接阻断（危险操作）
    Blocked { reason: String },
}

/// 用户确认的结果
enum ConfirmResult {
    Approved,
    Denied,
}

// ────────────────────────────────────────────────────────────
// Agent 结构体
// ────────────────────────────────────────────────────────────

/// Agent：将 LLM Provider 和工具系统组合在一起的核心结构
///
/// 持有三样东西：
/// 1. provider — 用于调用 LLM（已有的 Phase 1-2 基础设施）
/// 2. tool_registry — 管理可用工具（Phase 3 新增）
/// 3. messages — 完整对话历史（从 main.rs 的 Vec<Message> 升级为 Agent 内部管理）
pub struct Agent {
    /// LLM 提供者（Box<dyn LlmProvider> 支持运行时切换）
    provider: Box<dyn LlmProvider>,
    /// 工具注册表（存储所有可用工具）
    tool_registry: ToolRegistry,
    /// 对话历史（包含所有消息：system、user、assistant、tool）
    /// 为什么由 Agent 管理而不是外部传入？
    /// 因为 Agent Loop 会自动插入 tool_call 和 tool_result 消息，
    /// 外部调用者不需要关心这些内部细节
    messages: Vec<Message>,
}

impl Agent {
    /// 创建新的 Agent 实例
    ///
    /// 初始化时就构建系统提示词（基于已注册的工具）
    pub fn new(provider: Box<dyn LlmProvider>, tool_registry: ToolRegistry) -> Self {
        // 动态构建 system prompt，包含当前可用的工具描述
        let tool_defs = tool_registry.definitions();
        let system_prompt = prompt::build_system_prompt(&tool_defs);

        Self {
            provider,
            tool_registry,
            messages: vec![Message::system(system_prompt)],
        }
    }

    /// 运行一次完整的 Agent 交互
    ///
    /// 输入用户消息，经过"思考→行动→观察"循环后返回最终文本回复。
    /// 这是 Agent 的核心方法——内部实现了自主决策的工具调用循环。
    pub async fn run(&mut self, user_input: &str) -> Result<String> {
        // ── Step 1: 将用户消息加入对话历史 ──
        self.messages.push(Message::user(user_input));

        // 获取工具定义（每次都重新获取，为 Phase 8 动态工具注册预留）
        let tool_defs = self.tool_registry.definitions();

        // ── Step 2: Agent Loop ──
        let mut iterations = 0;

        loop {
            iterations += 1;

            // 安全阀：防止 LLM 陷入无限工具调用循环
            if iterations > MAX_ITERATIONS {
                let fallback =
                    "抱歉，我尝试了太多次工具调用但未能完成任务。请尝试简化你的问题。".to_string();
                self.messages.push(Message::assistant(&fallback));
                return Ok(fallback);
            }

            // ── 思考：调用 LLM，让它决定下一步动作 ──
            println!("  [Agent] 第 {} 轮思考中...", iterations);

            let response: LlmResponse = self
                .provider
                .chat_with_tools(&self.messages, &tool_defs)
                .await?;

            // ── 判断：LLM 是否想调用工具？──
            if !response.has_tool_calls() {
                // 没有工具调用 → LLM 给出了最终回答
                let final_text = response.content.unwrap_or_default();
                self.messages.push(Message::assistant(&final_text));
                return Ok(final_text);
            }

            // ── 行动 + 观察：执行工具并收集结果 ──

            // 1) 将 assistant 的工具调用决定记录到历史
            //    API 要求对话历史完整——后续请求必须包含之前的 tool_calls
            self.messages.push(Message::assistant_with_tool_calls(
                response.content.clone(),
                response.tool_calls.clone(),
            ));

            // 2) 如果 LLM 在调用工具前有思考文本，打印出来
            if let Some(ref thinking) = response.content {
                if !thinking.is_empty() {
                    println!("  [Agent 思考] {}", thinking);
                }
            }

            // 3) 依次执行每个工具调用（Phase 4：加入权限检查）
            for tool_call in &response.tool_calls {
                println!(
                    "  [工具调用] {}({})",
                    tool_call.name, tool_call.arguments
                );

                let result = self.execute_tool(tool_call).await;

                // 打印结果摘要（过长则截断，避免刷屏）
                let display_result = if result.len() > 200 {
                    format!("{}... [共 {} 字符]", &result[..200], result.len())
                } else {
                    result.clone()
                };
                println!("  [工具结果] {}", display_result);

                // 4) 将工具结果加入对话历史
                self.messages.push(Message::tool_result(
                    &tool_call.id,
                    &tool_call.name,
                    result,
                ));
            }

            // 回到循环顶部，让 LLM 基于工具结果继续推理
        }
    }

    // ────────────────────────────────────────────────────────
    // Phase 4: 权限检查 + 确认流程
    // ────────────────────────────────────────────────────────

    /// 执行单个工具调用（Phase 4：加入权限检查和确认流程）
    ///
    /// 流程：
    /// 1. 查找工具是否存在
    /// 2. 对需要权限检查的工具（bash, write_file），进行安全分类
    /// 3. 根据分类结果：自动执行 / 请求确认 / 阻断
    /// 4. 执行工具并返回结果
    ///
    /// 为什么确认逻辑在 Agent 层而不是 Tool 层？
    /// - Tool trait 保持纯净（只关心"执行"），方便测试和复用
    /// - 确认方式可能随 UI 变化（Phase 5 TUI），集中在 Agent 层便于替换
    /// - 不同工具的确认展示方式不同（bash 显示命令，write_file 显示路径+预览）
    ///
    /// 确认交互不进入 LLM 消息历史——纯终端 I/O。
    /// LLM 只看到 tool_result 是成功还是"已取消"。
    async fn execute_tool(&self, tool_call: &ToolCall) -> String {
        // 第一步：查找工具
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

        // 第二步：权限检查
        let action = self.check_permission(&tool_call.name, &tool_call.arguments);

        match action {
            PermissionAction::AutoExecute => {
                // 只读操作或无需检查的工具，直接执行
            }
            PermissionAction::NeedConfirmation { prompt_message } => {
                // 写操作：向用户展示确认提示
                match Self::ask_user_confirmation(&prompt_message) {
                    ConfirmResult::Approved => {
                        // 用户同意，继续执行
                    }
                    ConfirmResult::Denied => {
                        return "操作已被用户取消。".to_string();
                    }
                }
            }
            PermissionAction::Blocked { reason } => {
                // 危险操作：直接阻断
                return format!(
                    "⛔ 操作被安全策略阻止：{}\n该命令不会被执行。",
                    reason
                );
            }
        }

        // 第三步：执行工具
        match tool.execute(tool_call.arguments.clone()).await {
            Ok(result) => result,
            Err(e) => format!("工具执行出错: {}", e),
        }
    }

    /// 检查工具调用的权限等级，返回应采取的行动
    fn check_permission(&self, tool_name: &str, arguments: &Value) -> PermissionAction {
        match tool_name {
            "bash" => {
                let command = arguments["command"].as_str().unwrap_or("");
                let result = sandbox::classify(command);

                match result.level {
                    PermissionLevel::Read => PermissionAction::AutoExecute,
                    PermissionLevel::Write => PermissionAction::NeedConfirmation {
                        prompt_message: format!(
                            "🔧 即将执行写操作：\n   $ {}\n   分类原因：{}",
                            command, result.reason
                        ),
                    },
                    PermissionLevel::Dangerous => PermissionAction::Blocked {
                        reason: result.reason,
                    },
                }
            }
            "write_file" => {
                // write_file 始终需要确认
                let path = arguments["path"].as_str().unwrap_or("(未知路径)");
                let content = arguments["content"].as_str().unwrap_or("");
                let preview = content_preview(content, 3);

                PermissionAction::NeedConfirmation {
                    prompt_message: format!(
                        "📝 即将写入文件：{}\n   内容预览：\n{}",
                        path, preview
                    ),
                }
            }
            // read_file 等其他工具不需要确认
            _ => PermissionAction::AutoExecute,
        }
    }

    /// 向用户展示确认提示并获取回复
    ///
    /// 当前实现：直接使用 stdin/stdout（终端交互）
    /// Phase 5 TUI 改造时：改为通过 async channel 发送确认请求
    ///
    /// 为什么不把这个方法抽象成 trait？
    /// 现在只有一种实现（终端），提前抽象是过度设计（YAGNI）。
    /// Phase 5 需要时再提取 trait。
    fn ask_user_confirmation(message: &str) -> ConfirmResult {
        use std::io::{self, Write};

        // 黄色高亮显示确认提示
        println!("\n\x1b[33m{}\x1b[0m", message);
        print!("\x1b[33m   允许执行？[y/N] \x1b[0m");
        io::stdout().flush().unwrap_or(());

        // 读取用户输入
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return ConfirmResult::Denied; // 读取失败视为拒绝
        }

        let answer = input.trim().to_lowercase();
        if answer == "y" || answer == "yes" {
            ConfirmResult::Approved
        } else {
            // 默认拒绝（安全优先：空回车、'n'、'no'、其他任何输入都拒绝）
            ConfirmResult::Denied
        }
    }

    /// 获取对话历史的引用（用于调试、日志或 Phase 7 的会话保存）
    #[allow(dead_code)]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

// ────────────────────────────────────────────────────────────
// 辅助函数
// ────────────────────────────────────────────────────────────

/// 生成文件内容预览（前 N 行 + 省略提示）
///
/// 用于 write_file 确认提示中展示即将写入的内容
fn content_preview(content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let preview_lines: Vec<String> = lines
        .iter()
        .take(max_lines)
        .map(|line| format!("   │ {}", line))
        .collect();

    let mut preview = preview_lines.join("\n");

    if total > max_lines {
        preview.push_str(&format!("\n   │ ... (共 {} 行)", total));
    }

    preview
}
