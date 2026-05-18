/// TUI 模块：App 状态管理 + 事件循环
///
/// 架构：Elm 模式（Event → Update → View）
/// - Event：来自终端（键盘）或 Agent 任务（channel）
/// - Update：handle_key() / handle_agent_event() 修改 App 状态
/// - View：ui::render() 根据状态渲染界面
///
/// 异步设计：
/// - TUI 在主 async 任务中运行（持有 Terminal、raw mode）
/// - 使用 tokio::select! 同时监听终端事件和 Agent 消息
/// - crossterm EventStream 提供异步终端事件流

pub mod event;
pub mod ui;

use anyhow::Result;
use crossterm::{
    event::{Event as CrosstermEvent, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::{mpsc, oneshot};

use event::{AgentEvent, ChatEntry, ChatRole, InputMode, UserAction};

// ────────────────────────────────────────────────────────────
// App 状态
// ────────────────────────────────────────────────────────────

/// App 状态：TUI 的全部运行时数据
///
/// 为什么所有字段都在一个 struct 里？
/// - Elm 架构要求单一状态源（Single Source of Truth）
/// - render() 只需 &App 就能画出完整界面
/// - 状态变更集中在 handle_* 方法中，便于追踪和调试
pub struct App {
    /// 输入缓冲区（用户正在输入的文本）
    pub input_buffer: String,
    /// 当前输入模式
    pub input_mode: InputMode,
    /// 聊天历史记录
    pub chat_history: Vec<ChatEntry>,
    /// 是否应该退出
    pub should_quit: bool,

    /// 确认对话框的消息文本
    pub confirmation_msg: Option<String>,
    /// 确认对话框的响应通道（向 Agent 回传 y/n 结果）
    pub confirmation_tx: Option<oneshot::Sender<bool>>,

    /// 发送用户动作到 Agent 任务
    user_tx: mpsc::Sender<UserAction>,
}

impl App {
    fn new(user_tx: mpsc::Sender<UserAction>) -> Self {
        Self {
            input_buffer: String::new(),
            input_mode: InputMode::Normal,
            chat_history: Vec::new(),
            should_quit: false,
            confirmation_msg: None,
            confirmation_tx: None,
            user_tx,
        }
    }

    /// 处理键盘事件
    async fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        // Ctrl+C 在任何模式下都退出
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            let _ = self.user_tx.send(UserAction::Quit).await;
            self.should_quit = true;
            return;
        }

        match self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Enter => {
                    if !self.input_buffer.is_empty() {
                        let input = std::mem::take(&mut self.input_buffer);
                        // 在聊天历史中显示用户输入
                        self.chat_history.push(ChatEntry {
                            role: ChatRole::User,
                            content: input.clone(),
                        });
                        // 发送到 Agent 任务
                        let _ = self.user_tx.send(UserAction::Submit(input)).await;
                        self.input_mode = InputMode::WaitingForAgent;
                    }
                }
                KeyCode::Char(c) => {
                    self.input_buffer.push(c);
                }
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                }
                _ => {}
            },
            InputMode::WaitingForAgent => {
                // 等待 Agent 时忽略输入（Ctrl+C 已在上面处理）
            }
            InputMode::ConfirmationDialog => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(tx) = self.confirmation_tx.take() {
                        let _ = tx.send(true);
                    }
                    self.confirmation_msg = None;
                    self.input_mode = InputMode::WaitingForAgent;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    if let Some(tx) = self.confirmation_tx.take() {
                        let _ = tx.send(false);
                    }
                    self.confirmation_msg = None;
                    self.input_mode = InputMode::WaitingForAgent;
                }
                _ => {} // 确认对话框只响应 y/n/Esc
            },
        }
    }

    /// 处理来自 Agent 的事件
    fn handle_agent_event(&mut self, evt: AgentEvent) {
        match evt {
            AgentEvent::ThinkingStarted { iteration } => {
                self.chat_history.push(ChatEntry {
                    role: ChatRole::Status,
                    content: format!("第 {} 轮思考中...", iteration),
                });
            }
            AgentEvent::ThinkingContent(text) => {
                self.chat_history.push(ChatEntry {
                    role: ChatRole::Thinking,
                    content: text,
                });
            }
            AgentEvent::ToolCallStart { name, args } => {
                self.chat_history.push(ChatEntry {
                    role: ChatRole::ToolCall,
                    content: format!("{}({})", name, truncate_str(&args, 100)),
                });
            }
            AgentEvent::ToolCallResult(result) => {
                self.chat_history.push(ChatEntry {
                    role: ChatRole::ToolResult,
                    content: truncate_str(&result, 200),
                });
            }
            AgentEvent::FinalResponse(text) => {
                self.chat_history.push(ChatEntry {
                    role: ChatRole::Assistant,
                    content: text,
                });
                // Agent 完成，恢复输入模式
                self.input_mode = InputMode::Normal;
            }
            AgentEvent::Error(msg) => {
                self.chat_history.push(ChatEntry {
                    role: ChatRole::Error,
                    content: msg,
                });
                self.input_mode = InputMode::Normal;
            }
            AgentEvent::ConfirmationRequest { message, response_tx } => {
                self.confirmation_msg = Some(message);
                self.confirmation_tx = Some(response_tx);
                self.input_mode = InputMode::ConfirmationDialog;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────
// 主入口
// ────────────────────────────────────────────────────────────

/// 运行 TUI 应用
///
/// 这个函数会接管终端（raw mode + alternate screen），
/// 在退出时恢复终端状态（无论正常退出还是 panic）。
pub async fn run_app(
    user_tx: mpsc::Sender<UserAction>,
    mut agent_rx: mpsc::Receiver<AgentEvent>,
) -> Result<()> {
    // ── 终端初始化 ──
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // ── Panic hook：确保异常退出时恢复终端 ──
    // 为什么需要？如果程序 panic 而没恢复终端，
    // 用户的 shell 会处于 raw mode，无法正常使用
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // ── App 状态初始化 ──
    let mut app = App::new(user_tx);

    // 欢迎信息
    app.chat_history.push(ChatEntry {
        role: ChatRole::Status,
        content: "mini-buddy v0.5 — TUI 模式 (Ctrl+C 退出)".to_string(),
    });

    // ── crossterm 异步事件流 ──
    let mut term_events = EventStream::new();

    // ── 主事件循环 ──
    loop {
        // 渲染当前状态
        terminal.draw(|frame| ui::render(frame, &app))?;

        // 同时等待终端事件和 Agent 事件
        tokio::select! {
            // 终端事件（键盘、窗口大小变化）
            Some(Ok(evt)) = term_events.next() => {
                if let CrosstermEvent::Key(key) = evt {
                    app.handle_key(key).await;
                }
                // Resize 事件不需要处理——下次 draw 自动适配
            }
            // Agent 事件
            Some(agent_evt) = agent_rx.recv() => {
                app.handle_agent_event(agent_evt);
            }
        }

        if app.should_quit {
            break;
        }
    }

    // ── 清理：恢复终端状态 ──
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

/// 截断过长字符串（用于 UI 显示）
/// 使用字符数而非字节数截断，避免在 UTF-8 多字节字符中间切断
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}
