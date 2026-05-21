/// mini-buddy Phase 6 — 配置系统 + 多模型切换
///
/// 架构升级：
/// Phase 5: TUI 界面 + 异步架构
/// Phase 6: TOML 配置文件管理 Provider 设置，不再依赖硬编码环境变量
///
/// 配置文件：~/.mini-buddy/config.toml
/// 优先级：环境变量 > 配置文件 > 内置默认值
///
/// 下一步（Phase 7）：上下文管理 + 会话持久化

mod llm;
mod tools;
mod agent;
mod tui;
mod config;
mod context;
mod mcp;

use anyhow::Result;
use tokio::sync::mpsc;

use agent::Agent;
use tools::create_default_registry;
use tui::event::{AgentEvent, UserAction};

/// 根据配置创建 LLM Provider
///
/// Phase 6 变更：从配置文件加载 provider 设置，替代之前的纯环境变量方式。
/// 环境变量仍然可以覆盖配置文件中的值（优先级更高）。
fn create_provider(cfg: &config::Config) -> Result<Box<dyn llm::LlmProvider>> {
    let provider_cfg = cfg.active_provider()?;
    let provider_name = cfg.active_provider_name();

    match provider_cfg.provider_type.as_str() {
        "openai" => {
            let api_key = provider_cfg.resolve_api_key().unwrap_or_default();
            let base_url = provider_cfg.resolve_base_url();
            let model = provider_cfg.model.clone();

            Ok(Box::new(llm::openai::OpenAIProvider::new(
                api_key, base_url, model,
            )))
        }
        "anthropic" => {
            let api_key = provider_cfg.resolve_api_key().ok_or_else(|| {
                let env_hint = provider_cfg
                    .api_key_env
                    .as_deref()
                    .unwrap_or("ANTHROPIC_API_KEY");
                anyhow::anyhow!("请设置 {} 环境变量或在配置文件中填写 api_key", env_hint)
            })?;
            let model = provider_cfg.model.clone();
            let max_tokens = provider_cfg.max_tokens.unwrap_or(4096);

            Ok(Box::new(llm::anthropic::AnthropicProvider::new(
                api_key, model, max_tokens,
            )))
        }
        other => {
            anyhow::bail!(
                "Provider '{}' 的 type '{}' 不支持。支持的类型: openai, anthropic",
                provider_name, other
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. 加载配置 ─────────────────────────────────────────
    let cfg = config::load_config()?;
    let provider_name = cfg.active_provider_name();

    // ── 2. 初始化 Provider ───────────────────────────────────
    let provider = create_provider(&cfg)?;

    // ── 3. 创建 channel ─────────────────────────────────────
    let (user_tx, user_rx) = mpsc::channel::<UserAction>(8);
    let (agent_tx, agent_rx) = mpsc::channel::<AgentEvent>(64);

    // ── 4. 创建 Agent ───────────────────────────────────────
    let mut tool_registry = create_default_registry();
    
    // ── Phase 8: 注册 MCP 工具 ───────────────────────────────
    if let Err(e) = tools::register_mcp_tools(&mut tool_registry, &cfg).await {
        agent::log_info(&format!("⚠ MCP 工具注册失败: {}", e));
    }
    
    let agent = Agent::new(provider, tool_registry, agent_tx);

    // ── 5. 启动 Agent task ──────────────────────────────────
    tokio::spawn(async move {
        agent_task(agent, user_rx).await;
    });

    // ── 6. 运行 TUI ─────────────────────────────────────────
    // 在 TUI 启动前打印配置信息到日志
    agent::log_info(&format!(
        "启动 mini-buddy | provider={} | config={}",
        provider_name,
        config::config_path().display()
    ));

    tui::run_app(user_tx, agent_rx).await?;

    Ok(())
}

/// Agent 后台任务
async fn agent_task(mut agent: Agent, mut user_rx: mpsc::Receiver<UserAction>) {
    while let Some(action) = user_rx.recv().await {
        match action {
            UserAction::Submit(input) => {
                if let Err(e) = agent.run(&input).await {
                    eprintln!("Agent error: {}", e);
                }
            }
            UserAction::Command(cmd) => {
                handle_command(&mut agent, &cmd).await;
            }
            UserAction::Quit => break,
        }
    }
}

/// 处理斜杠命令
async fn handle_command(agent: &mut Agent, cmd: &str) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    let command = parts[0];

    match command {
        "/save" => {
            match context::storage::save_session(agent.get_messages()) {
                Ok(meta) => {
                    agent::log_info(&format!("会话已保存: {} ({})", meta.title, meta.id));
                }
                Err(e) => {
                    agent::log_info(&format!("保存会话失败: {}", e));
                }
            }
        }
        "/load" => {
            match context::storage::load_latest_session() {
                Ok(Some(messages)) => {
                    let count = messages.len();
                    agent.set_messages(messages);
                    agent::log_info(&format!("已加载最近会话 ({} 条消息)", count));
                }
                Ok(None) => {
                    agent::log_info("没有已保存的会话");
                }
                Err(e) => {
                    agent::log_info(&format!("加载会话失败: {}", e));
                }
            }
        }
        "/new" => {
            // 保存当前会话再重置
            let _ = context::storage::save_session(agent.get_messages());
            agent.set_messages(Vec::new());
            agent::log_info("新会话已开始（旧会话已自动保存）");
        }
        _ => {
            agent::log_info(&format!("未知命令: {}", cmd));
        }
    }
}
