/// 系统提示词构建器
///
/// Agent 的行为很大程度上取决于系统提示词的质量。
/// 好的 system prompt 让 LLM 知道：
/// 1. 它是什么角色（编程助手）
/// 2. 它有哪些能力（可用工具列表）
/// 3. 应该如何行动（工具使用策略）
///
/// 为什么动态构建而不是硬编码？
/// 因为工具列表是可配置的（Phase 8 MCP 会动态添加工具），
/// prompt 需要实时反映当前可用的工具集合。

use crate::llm::ToolDefinition;

/// 构建 Agent 的系统提示词
///
/// 虽然工具定义已经通过 API 参数传给了 LLM（tools 数组），
/// 但在 system prompt 中再次描述可以增强 LLM 的工具使用倾向，
/// 并提供使用策略的指导——这是 Prompt Engineering 的常见技巧。
pub fn build_system_prompt(tools: &[ToolDefinition]) -> String {
    let mut prompt = String::new();

    // ── 角色定义 ──
    prompt.push_str(
        "你是一个智能编程助手。你可以通过调用工具来帮助用户完成任务。\n\n",
    );

    // ── 工具使用策略 ──
    prompt.push_str("## 工具使用原则\n\n");
    prompt.push_str("1. 当用户的问题需要查看文件系统或执行命令时，主动使用工具\n");
    prompt.push_str("2. 先思考需要什么信息，再决定调用哪个工具\n");
    prompt.push_str("3. 如果一个工具的结果不够，可以连续调用多个工具\n");
    prompt.push_str("4. 获得工具结果后，用自然语言总结发现并回答用户\n");
    prompt.push_str("5. 如果工具执行失败，分析错误原因并尝试其他方法\n\n");

    // ── 可用工具列表 ──
    if !tools.is_empty() {
        prompt.push_str("## 可用工具\n\n");
        for tool in tools {
            prompt.push_str(&format!("- **{}**: {}\n", tool.name, tool.description));
        }
    }

    prompt
}
