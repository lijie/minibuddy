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
/// 执行工具    返回最终回答           │
///   │                               │
///   ▼                               │
/// 将结果加入历史 ───────────────────┘
/// ```
///
/// 关键设计决策：
/// - 使用非流式调用（chat_with_tools）：需要完整响应才能判断是否有工具调用
/// - 最大循环次数限制：防止 LLM 陷入无限工具调用
/// - 错误宽容：工具执行失败不终止循环，而是将错误信息反馈给 LLM

pub mod prompt;

use anyhow::Result;

use crate::llm::{LlmProvider, LlmResponse, Message, ToolCall};
use crate::tools::ToolRegistry;

/// 最大工具调用轮次（安全阀，防止无限循环）
///
/// 大多数任务 2-3 轮就够了（查找→读取→回答），
/// 10 轮是足够宽裕的上限。如果 10 轮还没完成，
/// 很可能是 LLM 陷入了循环或任务本身有问题。
const MAX_ITERATIONS: usize = 10;

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

            // 3) 依次执行每个工具调用
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

    /// 执行单个工具调用
    ///
    /// 设计原则：宽容处理错误
    /// - 工具不存在 → 返回友好错误信息（LLM 通常会自行纠正）
    /// - 工具执行出错 → 返回错误信息（LLM 可以基于错误调整策略）
    /// - 绝不 panic — 错误信息也是对 LLM 有价值的"观察"
    async fn execute_tool(&self, tool_call: &ToolCall) -> String {
        match self.tool_registry.get(&tool_call.name) {
            Some(tool) => {
                match tool.execute(tool_call.arguments.clone()).await {
                    Ok(result) => result,
                    Err(e) => {
                        // 工具执行出错：将错误信息返回给 LLM
                        format!("工具执行出错: {}", e)
                    }
                }
            }
            None => {
                // LLM 幻觉了一个不存在的工具名
                // 列出可用工具，帮助 LLM 自行纠正
                let available: Vec<String> = self
                    .tool_registry
                    .definitions()
                    .iter()
                    .map(|d| d.name.clone())
                    .collect();
                format!(
                    "错误：工具 '{}' 不存在。可用工具: {:?}",
                    tool_call.name, available
                )
            }
        }
    }

    /// 获取对话历史的引用（用于调试、日志或 Phase 7 的会话保存）
    #[allow(dead_code)]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}
