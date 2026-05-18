/// mini-buddy Phase 3 — Agent Loop + Tool Use
///
/// 架构升级：
/// Phase 1: stdin → Message → chat() → stdout（单轮，非流式）
/// Phase 2: loop { stdin → history.push() → chat_stream() → 逐 token stdout }（多轮，流式）
/// Phase 3: Agent { Provider + ToolRegistry + History } → 思考→行动→观察 循环
///
/// 核心变化：
/// - 引入 Agent 结构体，封装 Provider + 工具系统 + 对话历史
/// - LLM 可以自主决定调用工具（bash、read_file）
/// - 工具执行结果自动反馈给 LLM，形成闭环
/// - 从简单"对话"升级为自主"代理"
///
/// 下一步（Phase 4）：Bash 沙盒 + 权限控制

mod llm;
mod tools;
mod agent;

use anyhow::Result;
use std::io::{self, BufRead, Write};

use agent::Agent;
use tools::create_default_registry;

/// 根据环境变量创建对应的 LLM Provider
///
/// 为什么用工厂函数而不是在 main 里 match？
/// - 集中 Provider 注册逻辑，新增 Provider 只改这一处
/// - 返回 trait object（Box<dyn LlmProvider>），调用方无需知道具体类型
/// - Phase 6 引入配置文件后，这个函数会改为从配置读取
fn create_provider() -> Result<Box<dyn llm::LlmProvider>> {
    // LLM_PROVIDER 环境变量控制使用哪个服务
    // 不设置时默认 deepseek，覆盖面最广
    let provider_name = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());

    match provider_name.as_str() {
        "deepseek" => {
            let api_key = std::env::var("DEEPSEEK_API_KEY")
                .map_err(|_| anyhow::anyhow!(
                    "❌ 请设置 DEEPSEEK_API_KEY 环境变量：export DEEPSEEK_API_KEY=\"your-key\""
                ))?;
            Ok(Box::new(llm::openai::OpenAIProvider::deepseek(api_key)))
        }
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!(
                    "❌ 请设置 ANTHROPIC_API_KEY 环境变量：export ANTHROPIC_API_KEY=\"your-key\""
                ))?;
            Ok(Box::new(llm::anthropic::AnthropicProvider::claude_sonnet(api_key)))
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

    // ── 2. 初始化 Agent（Phase 3 核心变化）────────────────────
    // Agent = Provider + ToolRegistry + 对话历史
    // 替代了 Phase 2 中 main.rs 直接管理 Vec<Message> 的方式
    let tool_registry = create_default_registry();
    let mut agent = Agent::new(provider, tool_registry);

    // ── 3. 打印欢迎信息 ─────────────────────────────────────
    let provider_name = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());
    println!("🤖 mini-buddy v0.3 — Agent Loop + Tool Use");
    println!("   Provider: {}  (输入 /quit 退出，Ctrl+D 也可以)", provider_name);
    println!("   试试问我：当前目录有什么文件？\n");

    // ── 4. 对话主循环 ───────────────────────────────────────
    let stdin = io::stdin();
    let mut reader = stdin.lock().lines();

    loop {
        // 打印提示符
        print!(">>> ");
        io::stdout().flush()?;

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

        // ── 调用 Agent Loop ──────────────────────────────────
        // Phase 2：直接调用 chat_stream() → 逐 token 输出
        // Phase 3：调用 agent.run() → 内部自动执行工具调用循环 → 返回最终回答
        //
        // Agent.run() 内部会：
        // 1. 将用户消息加入历史
        // 2. 调用 LLM（带工具定义）
        // 3. 如果 LLM 想调用工具 → 执行 → 将结果反馈 → 重复
        // 4. 直到 LLM 给出最终文本回答
        match agent.run(&input).await {
            Ok(response) => {
                println!("\n{}\n", response);
            }
            Err(e) => {
                eprintln!("\n❌ 错误: {}\n", e);
            }
        }
    }

    println!("\n👋 再见！");
    Ok(())
}
