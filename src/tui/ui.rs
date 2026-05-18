/// TUI 布局渲染
///
/// 使用 Ratatui 的 widget 系统渲染界面。
/// 布局：上下两分栏（对话历史 + 输入区）。
///
/// Phase 5 基础版特性：
/// - 按 ChatRole 着色
/// - 自动滚动到底部
/// - 确认对话框覆盖输入区
///
/// Phase 9 将新增：Markdown 渲染、代码高亮、主题、滚动控制

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::event::{ChatEntry, ChatRole, InputMode};
use super::App;

/// 渲染整个界面
///
/// 为什么是纯函数（&App → Frame 副作用）？
/// Elm 架构：View 只负责"根据状态画图"，不修改状态。
/// 状态变更在 handle_key / handle_agent_event 中完成。
pub fn render(frame: &mut Frame, app: &App) {
    // ── 布局分割：聊天区（弹性）+ 输入区（固定 3 行）──
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // 聊天历史区（占满剩余空间）
            Constraint::Length(3), // 输入区（边框 + 一行文本 + 边框 = 3）
        ])
        .split(frame.area());

    // ── 聊天历史区 ──
    let history_items: Vec<ListItem> = app
        .chat_history
        .iter()
        .map(|entry| render_chat_entry(entry))
        .collect();

    let history_widget = List::new(history_items)
        .block(Block::default().borders(Borders::ALL).title(" mini-buddy "));

    frame.render_widget(history_widget, chunks[0]);

    // ── 输入区 ──
    let (input_title, input_content, input_style) = match app.input_mode {
        InputMode::Normal => (
            " Input (Enter 发送, Ctrl+C 退出) ",
            app.input_buffer.as_str(),
            Style::default(),
        ),
        InputMode::WaitingForAgent => (
            " ⏳ Agent 处理中... ",
            "",
            Style::default().fg(Color::DarkGray),
        ),
        InputMode::ConfirmationDialog => (
            " ⚠️  确认操作 (y/n) ",
            app.confirmation_msg.as_deref().unwrap_or("允许执行？"),
            Style::default().fg(Color::Yellow),
        ),
    };

    let input_widget = Paragraph::new(input_content)
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .style(input_style);

    frame.render_widget(input_widget, chunks[1]);

    // ── 光标位置 ──
    // 只在正常输入模式下显示光标
    // 使用 unicode 显示宽度而非字节数——中文字符占 2 列宽
    if app.input_mode == InputMode::Normal {
        let cursor_x = app.input_buffer.width() as u16;
        frame.set_cursor_position((
            chunks[1].x + cursor_x + 1, // +1 左边框
            chunks[1].y + 1,             // +1 上边框
        ));
    }
}

/// 将单条 ChatEntry 渲染为 ListItem（支持多行内容）
fn render_chat_entry(entry: &ChatEntry) -> ListItem<'static> {
    let (prefix, color) = match entry.role {
        ChatRole::User => (">>> ", Color::Cyan),
        ChatRole::Assistant => ("", Color::White),
        ChatRole::ToolCall => ("  🔧 ", Color::Yellow),
        ChatRole::ToolResult => ("  ← ", Color::DarkGray),
        ChatRole::Thinking => ("  💭 ", Color::Magenta),
        ChatRole::Status => ("  ⏳ ", Color::Blue),
        ChatRole::Error => ("  ❌ ", Color::Red),
    };

    let style = Style::default().fg(color);

    // 多行内容：将每行包装为 Line，第一行带前缀
    let content_lines: Vec<&str> = entry.content.lines().collect();

    let lines: Vec<Line<'static>> = if content_lines.is_empty() {
        vec![Line::from(Span::styled(prefix.to_string(), style))]
    } else {
        content_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let text = if i == 0 {
                    format!("{}{}", prefix, line)
                } else {
                    // 后续行缩进对齐（前缀等宽空格）
                    format!("{}{}", " ".repeat(prefix.chars().count()), line)
                };
                Line::from(Span::styled(text, style))
            })
            .collect()
    };

    ListItem::new(lines)
}
