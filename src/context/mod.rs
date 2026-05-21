/// 上下文管理：Token 估算 + 截断策略
///
/// 问题：对话历史会无限增长，超出 LLM 的上下文窗口（如 DeepSeek 32K、Claude 200K）。
///
/// 解决方案：
/// 1. 估算当前消息列表的 token 数
/// 2. 超限时截断最旧的消息（保留 system prompt + 最近 N 条）
///
/// 为什么用简单字符估算而非 tiktoken？
/// - 教学项目，避免引入重量级依赖
/// - 字符估算的误差（±20%）对截断决策影响不大——我们留了足够 buffer
/// - 真正精确的 token 计数需要知道具体 tokenizer（各模型不同）

pub mod storage;

use crate::llm::Message;

/// 上下文管理器
pub struct ContextManager {
    /// 允许的最大 token 数（发送给 LLM 的消息总量上限）
    /// 留出模型回复空间：如果模型支持 32K，这里设 24K
    max_tokens: usize,
}

impl ContextManager {
    pub fn new(max_tokens: usize) -> Self {
        Self { max_tokens }
    }

    /// 默认配置：8000 token 上限
    /// 适用于大多数模型（DeepSeek、GPT-4o 等都至少 8K）
    /// Phase 6 的配置系统可以让用户自定义这个值
    pub fn default() -> Self {
        Self::new(8000)
    }

    /// 估算单段文本的 token 数
    ///
    /// 规则（教学级简化）：
    /// - 中文/日文/韩文字符：每字符 ≈ 0.5 token（实际约 0.5-0.7）
    /// - ASCII 字符：每 4 字符 ≈ 1 token（英文平均词长约 4-5）
    /// - 取整时向上取——宁可高估也不低估
    pub fn estimate_tokens(text: &str) -> usize {
        let mut cjk_chars = 0;
        let mut ascii_chars = 0;

        for c in text.chars() {
            if c.is_ascii() {
                ascii_chars += 1;
            } else {
                // CJK 及其他非 ASCII 字符（表情符号等）
                cjk_chars += 1;
            }
        }

        // 中文：约 2 字符 = 1 token → chars / 2 + 1
        // ASCII：约 4 字符 = 1 token → chars / 4 + 1
        let cjk_tokens = (cjk_chars + 1) / 2;
        let ascii_tokens = (ascii_chars + 3) / 4;

        cjk_tokens + ascii_tokens
    }

    /// 估算单条消息的 token 数
    /// 每条消息有固定开销（role 标记、分隔符等约 4 token）
    pub fn estimate_message_tokens(msg: &Message) -> usize {
        const MESSAGE_OVERHEAD: usize = 4;

        let content_tokens = Self::estimate_tokens(&msg.content);

        // tool_calls 中的参数也占 token
        let tool_call_tokens = msg
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls.iter().map(|tc| {
                    Self::estimate_tokens(&tc.name) + Self::estimate_tokens(&tc.arguments.to_string())
                }).sum::<usize>()
            })
            .unwrap_or(0);

        MESSAGE_OVERHEAD + content_tokens + tool_call_tokens
    }

    /// 估算消息列表的总 token 数
    pub fn estimate_total_tokens(messages: &[Message]) -> usize {
        messages.iter().map(|m| Self::estimate_message_tokens(m)).sum()
    }

    /// 如果超限则截断消息
    ///
    /// 截断策略：保留第一条消息（system prompt）+ 尽可能多的最近消息
    /// 为什么从前面截断而非后面？
    /// - 最近的消息对当前对话最相关
    /// - system prompt 必须保留（定义了 Agent 行为）
    /// - 最旧的对话轮次对当前任务帮助最小
    ///
    /// 返回被截断的消息数量（0 表示未截断）
    pub fn truncate_if_needed(&self, messages: &mut Vec<Message>) -> usize {
        if messages.len() <= 2 {
            return 0; // system + 最近一条，不需要截断
        }

        let total = Self::estimate_total_tokens(messages);
        if total <= self.max_tokens {
            return 0;
        }

        // 策略：保留 messages[0]（system prompt）+ 从后往前保留尽可能多的消息
        let system_tokens = Self::estimate_message_tokens(&messages[0]);
        let budget = self.max_tokens.saturating_sub(system_tokens);

        // 从后往前累积，找到可以保留的起始位置
        let mut keep_from = messages.len(); // 从这个 index 开始保留
        let mut accumulated = 0;

        for i in (1..messages.len()).rev() {
            let msg_tokens = Self::estimate_message_tokens(&messages[i]);
            if accumulated + msg_tokens > budget {
                break;
            }
            accumulated += msg_tokens;
            keep_from = i;
        }

        // 确保至少保留最后一条消息
        if keep_from >= messages.len() {
            keep_from = messages.len() - 1;
        }

        let removed_count = keep_from - 1; // -1 因为 messages[0] 始终保留

        if removed_count > 0 {
            // 保留 system prompt (index 0) + 从 keep_from 到末尾的消息
            let retained_tail: Vec<Message> = messages[keep_from..].to_vec();
            messages.truncate(1); // 只保留 system prompt
            messages.extend(retained_tail);
        }

        removed_count
    }
}

// ────────────────────────────────────────────────────────────
// 单元测试
// ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Message, Role};

    #[test]
    fn test_estimate_tokens_ascii() {
        // "hello" = 5 ASCII chars → ceil(5/4) = 2 tokens
        let tokens = ContextManager::estimate_tokens("hello");
        assert!(tokens >= 1 && tokens <= 3);
    }

    #[test]
    fn test_estimate_tokens_chinese() {
        // "你好世界" = 4 CJK chars → ceil(4/2) = 2 tokens
        let tokens = ContextManager::estimate_tokens("你好世界");
        assert!(tokens >= 2 && tokens <= 4);
    }

    #[test]
    fn test_estimate_tokens_mixed() {
        // "Hello 你好" = 6 ASCII + 2 CJK
        let tokens = ContextManager::estimate_tokens("Hello 你好");
        assert!(tokens >= 2 && tokens <= 5);
    }

    #[test]
    fn test_no_truncation_when_under_limit() {
        let mut messages = vec![
            Message::system("你是助手"),
            Message::user("你好"),
            Message::assistant("你好！"),
        ];
        let cm = ContextManager::new(10000);
        let removed = cm.truncate_if_needed(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn test_truncation_keeps_system_and_recent() {
        let mut messages = vec![Message::system("system prompt")];
        // 添加很多消息使其超限
        for i in 0..100 {
            messages.push(Message::user(&format!("消息 {} 这是一段比较长的文本用来占位", i)));
            messages.push(Message::assistant(&format!("回复 {} 这也是一段比较长的回复文本", i)));
        }

        let cm = ContextManager::new(500); // 很小的限制
        let removed = cm.truncate_if_needed(&mut messages);

        assert!(removed > 0);
        // system prompt 仍在第一条
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("system prompt"));
        // 总量应该在限制范围内
        let total = ContextManager::estimate_total_tokens(&messages);
        assert!(total <= 500 + 50); // 允许一点误差（最后加入的消息可能略超）
    }
}
