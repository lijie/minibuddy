/// mini-buddy Phase 1 — 最简单轮 LLM 问答
///
/// 架构：stdin → Message → LlmProvider::chat() → stdout
/// 下一步（Phase 2）：维护 Vec<Message> 历史，实现多轮对话 + 流式输出

mod llm;

use anyhow::{Context, Result};
use llm::openai::OpenAIProvider;
use llm::{LlmProvider, Message};
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. 读取 API Key ──────────────────────────────────────
    // 设计原因：从环境变量读取，绝不硬编码，避免密钥进入版本控制
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .context("❌ 请先设置环境变量：export DEEPSEEK_API_KEY=\"your-key\"")?;

    // ── 2. 初始化 Provider ───────────────────────────────────
    // 使用 DeepSeek 作为 OpenAI 兼容接口的默认 Provider
    // 换其他服务（Qwen/Kimi）只需调用 OpenAIProvider::new() 传入对应 base_url 和 model
    let provider = OpenAIProvider::deepseek(api_key);

    println!("🤖 mini-buddy Phase 1 — 单轮问答");
    println!("   模型: deepseek-chat  (Ctrl+C 退出)\n");

    // ── 3. 读取用户输入 ──────────────────────────────────────
    print!("You: ");
    io::stdout().flush()?; // print! 不自动 flush，需要手动刷新

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let input = input.trim();
    if input.is_empty() {
        println!("（输入为空，退出）");
        return Ok(());
    }

    // ── 4. 构造消息并调用 LLM ────────────────────────────────
    // Phase 1 只有一条用户消息；Phase 2 会改成 Vec<Message> 历史
    let messages = vec![Message::user(input)];

    print!("\nAssistant: ");
    io::stdout().flush()?;

    let reply = provider.chat(&messages).await?;

    // ── 5. 打印回复 ──────────────────────────────────────────
    println!("{reply}");

    Ok(())
}
