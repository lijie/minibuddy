/// mini-buddy Phase 5 — TUI 基础界面
///
/// 架构升级：
/// Phase 1-4: 裸 stdin/stdout 对话（阻塞式）
/// Phase 5:   Ratatui TUI + 异步架构（UI task ↔ Agent task 通过 channel 通信）
///
/// 核心变化：
/// - UI 和 Agent 运行在不同的 async task 中
/// - 通过 tokio::sync::mpsc channel 双向通信
/// - TUI 使用 Elm 架构（Event → Update → View）
/// - 工具调用状态实时显示在界面中
///
/// 下一步（Phase 6）：配置系统 + 多模型切换

mod llm;
mod tools;
mod agent;
mod tui;

use anyhow::Result;
use tokio::sync::mpsc;

use agent::Agent;
use tools::create_default_registry;
use tui::event::{AgentEvent, UserAction};

/// 根据环境变量创建对应的 LLM Provider
fn create_provider() -> Result<Box<dyn llm::LlmProvider>> {
    let provider_name = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string());

    match provider_name.as_str() {
        "deepseek" => {
            let api_key = std::env::var("DEEPSEEK_API_KEY")
                .map_err(|_| anyhow::anyhow!(
                    "请设置 DEEPSEEK_API_KEY 环境变量"
                ))?;
            Ok(Box::new(llm::openai::OpenAIProvider::deepseek(api_key)))
        }
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!(
                    "请设置 ANTHROPIC_API_KEY 环境变量"
                ))?;
            Ok(Box::new(llm::anthropic::AnthropicProvider::claude_sonnet(api_key)))
        }
        "ollama" => {
            let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen2.5".to_string());
            Ok(Box::new(llm::openai::OpenAIProvider::ollama(model)))
        }
        other => {
            anyhow::bail!(
                "未知的 LLM Provider: '{}'。支持: deepseek, anthropic, ollama",
                other
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. 初始化 Provider ───────────────────────────────────
    let provider = create_provider()?;

    // ── 2. 创建 channel ─────────────────────────────────────
    // user_tx/rx: UI → Agent（用户输入）
    // agent_tx/rx: Agent → UI（状态事件）
    let (user_tx, user_rx) = mpsc::channel::<UserAction>(8);
    let (agent_tx, agent_rx) = mpsc::channel::<AgentEvent>(64);

    // ── 3. 创建 Agent ───────────────────────────────────────
    let tool_registry = create_default_registry();
    let agent = Agent::new(provider, tool_registry, agent_tx);

    // ── 4. 启动 Agent task ──────────────────────────────────
    // Agent 在独立 task 中运行，不阻塞 UI
    tokio::spawn(async move {
        agent_task(agent, user_rx).await;
    });

    // ── 5. 运行 TUI（阻塞主 task 直到退出）─────────────────
    tui::run_app(user_tx, agent_rx).await?;

    Ok(())
}

/// Agent 后台任务：循环等待用户输入，执行 Agent Loop
///
/// 为什么单独抽出来而不是在 main 里写？
/// - 清晰的职责边界：这个函数只关心"收到输入 → 跑 Agent"
/// - Agent 的生命周期由这个 task 管理
/// - 未来可以在这里加重试、超时等逻辑
async fn agent_task(mut agent: Agent, mut user_rx: mpsc::Receiver<UserAction>) {
    while let Some(action) = user_rx.recv().await {
        match action {
            UserAction::Submit(input) => {
                if let Err(e) = agent.run(&input).await {
                    // 发送错误事件到 TUI（agent 内部的 event_tx 可能已关闭）
                    // 这里的错误是系统级错误（如网络断开），不是工具执行错误
                    eprintln!("Agent error: {}", e);
                }
            }
            UserAction::Quit => break,
        }
    }
}
