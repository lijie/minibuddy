/// mini-buddy Phase 2 — 多轮对话 + 流式输出
///
/// 架构升级：
/// Phase 1: stdin → Message → chat() → stdout（单轮，非流式）
/// Phase 2: loop { stdin → history.push() → chat_stream() → 逐 token stdout }（多轮，流式）
///
/// 新增能力：
/// - 维护 Vec<Message> 对话历史，实现多轮上下文
/// - 流式逐 token 输出（打字机效果）
/// - 支持 DeepSeek 和 Anthropic 两种 Provider，通过环境变量切换
///
/// 下一步（Phase 3）：Agent Loop + Tool Use

mod llm;

use anyhow::Result;
use futures::StreamExt;
use llm::openai::OpenAIProvider;
use llm::anthropic::AnthropicProvider;
use llm::{LlmProvider, Message, StreamChunk};
use std::io::{self, BufRead, Write};

/// 根据环境变量创建对应的 LLM Provider
///
/// 为什么用工厂函数而不是在 main 里 match？
/// - 集中 Provider 注册逻辑，新增 Provider 只改这一处
/// - 返回 trait object（Box<dyn LlmProvider>），调用方无需知道具体类型
/// - Phase 6 引入配置文件后，这个函数会改为从配置读取
fn create_provider() -> Result<Box<dyn LlmProvider>> {
    // LLM_PROVIDER 环境变量控制使用哪个服务
    // 不设置时默认 deepseek，覆盖面最广
    let provider_name = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());

    match provider_name.as_str() {
        "deepseek" => {
            let api_key = std::env::var("DEEPSEEK_API_KEY")
                .map_err(|_| anyhow::anyhow!(
                    "❌ 请设置 DEEPSEEK_API_KEY 环境变量：export DEEPSEEK_API_KEY=\"your-key\""
                ))?;
            Ok(Box::new(OpenAIProvider::deepseek(api_key)))
        }
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!(
                    "❌ 请设置 ANTHROPIC_API_KEY 环境变量：export ANTHROPIC_API_KEY=\"your-key\""
                ))?;
            Ok(Box::new(AnthropicProvider::claude_sonnet(api_key)))
        }
        other => {
            anyhow::bail!(
                "❌ 未知的 LLM Provider: '{}'。支持的选项: deepseek, anthropic\n\
                 设置方式: export LLM_PROVIDER=\"deepseek\"",
                other
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. 初始化 Provider ───────────────────────────────────
    let provider = create_provider()?;

    // ── 2. 初始化对话历史 ────────────────────────────────────
    // 为什么用 Vec 而不是 VecDeque？
    // - 每次 API 调用需要完整有序历史，Vec 的连续内存布局最优
    // - Phase 7 如需截断（context window 管理），从前端 drain 即可
    let mut history: Vec<Message> = Vec::new();

    // 添加 system prompt — 设定助手角色
    // Phase 6 引入配置系统后，这里会改为从配置文件读取
    history.push(Message::system(
        "你是一个有帮助的编程助手。回答简洁明了。",
    ));

    // ── 3. 打印欢迎信息 ─────────────────────────────────────
    let provider_name = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());
    println!("🤖 mini-buddy v0.2 — 多轮对话 + 流式输出");
    println!("   Provider: {}  (输入 /quit 退出，Ctrl+D 也可以)\n", provider_name);

    // ── 4. 对话主循环 ───────────────────────────────────────
    let stdin = io::stdin();
    let mut reader = stdin.lock().lines();

    loop {
        // 打印提示符
        print!(">>> ");
        io::stdout().flush()?; // print! 不自动 flush，需要手动刷新

        // 读取用户输入
        let line = match reader.next() {
            Some(Ok(line)) => line,
            Some(Err(e)) => return Err(e.into()),
            None => break, // EOF（Ctrl+D）
        };

        let input = line.trim().to_string();

        // 空行跳过
        if input.is_empty() {
            continue;
        }

        // 退出命令
        if input == "/quit" || input == "/exit" {
            break;
        }

        // ── 4a. 追加用户消息到历史 ──────────────────────────
        history.push(Message::user(&input));

        // ── 4b. 流式调用 LLM ────────────────────────────────
        // 注意借用规则：chat_stream() 不可变借用 &history，
        // 而后面 history.push() 需要可变借用。
        // 解决方案：用 block 限定 stream 的作用域，确保在 push 前 stream 已被 drop
        let full_response = {
            let mut stream = provider.chat_stream(&history);
            let mut response = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(StreamChunk::Delta(text)) => {
                        // 逐 token 打印 — 打字机效果
                        // 关键：必须 flush，否则 print! 的内容会攒到换行才输出
                        print!("{}", text);
                        io::stdout().flush()?;
                        response.push_str(&text);
                    }
                    Ok(StreamChunk::Done) => {
                        break;
                    }
                    Err(e) => {
                        eprintln!("\n❌ 流式输出中断: {}", e);
                        break;
                    }
                }
            }

            response
        }; // stream 在这里被 drop，释放对 history 的不可变借用

        // 响应结束后换行
        println!();

        // ── 4c. 追加助手消息到历史 ──────────────────────────
        // 用完整响应文本，不是单个 chunk
        // TODO: Phase 7 — 上下文窗口管理，防止历史无限增长
        if !full_response.is_empty() {
            history.push(Message::assistant(&full_response));
        }
    }

    println!("\n👋 再见！");
    Ok(())
}
