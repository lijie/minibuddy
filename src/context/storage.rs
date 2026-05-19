/// 会话持久化：JSON 文件存储
///
/// 存储路径：~/.mini-buddy/sessions/{id}.json
/// 格式：{ meta: {...}, messages: [...] }
///
/// 为什么用 JSON 而非 SQLite？
/// - 教学项目，零额外依赖（serde_json 已有）
/// - 每个会话独立文件，可直接用文本编辑器查看/调试
/// - 文件数不会太多（个人使用），不需要索引查询能力

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::llm::{Message, Role, ToolCall};

// ────────────────────────────────────────────────────────────
// 会话元数据
// ────────────────────────────────────────────────────────────

/// 会话元数据（存储在 JSON 文件中 + 用于列表展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub message_count: usize,
}

/// 完整会话文件格式
#[derive(Serialize, Deserialize)]
struct SessionFile {
    meta: SessionMeta,
    messages: Vec<StoredMessage>,
}

/// 可序列化的消息格式（因为 Message 没有 derive Serialize）
#[derive(Serialize, Deserialize)]
struct StoredMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<StoredToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredToolCall {
    id: String,
    name: String,
    arguments: Value,
}

// ────────────────────────────────────────────────────────────
// 路径
// ────────────────────────────────────────────────────────────

/// 获取会话存储目录
fn sessions_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".mini-buddy").join("sessions")
}

// ────────────────────────────────────────────────────────────
// 公共 API
// ────────────────────────────────────────────────────────────

/// 保存会话到 JSON 文件
pub fn save_session(messages: &[Message]) -> Result<SessionMeta> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建会话目录失败: {}", dir.display()))?;

    // 生成会话 ID（时间戳格式，人类可读）
    let id = generate_session_id();

    // 提取标题：第一条 user 消息的前 30 字符
    let title = messages
        .iter()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.chars().take(30).collect::<String>())
        .unwrap_or_else(|| "新会话".to_string());

    let meta = SessionMeta {
        id: id.clone(),
        title,
        created_at: current_timestamp(),
        message_count: messages.len(),
    };

    // 转换为可序列化格式
    let stored_messages: Vec<StoredMessage> = messages.iter().map(message_to_stored).collect();

    let session_file = SessionFile {
        meta: meta.clone(),
        messages: stored_messages,
    };

    let path = dir.join(format!("{}.json", id));
    let content = serde_json::to_string_pretty(&session_file)
        .context("序列化会话失败")?;
    std::fs::write(&path, content)
        .with_context(|| format!("写入会话文件失败: {}", path.display()))?;

    Ok(meta)
}

/// 列出所有已保存的会话（按创建时间倒序）
pub fn list_sessions() -> Result<Vec<SessionMeta>> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(file) = serde_json::from_str::<SessionFile>(&content) {
                    sessions.push(file.meta);
                }
            }
        }
    }

    // 按 ID（时间戳）倒序
    sessions.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(sessions)
}

/// 加载指定会话的消息
pub fn load_session(id: &str) -> Result<Vec<Message>> {
    let path = sessions_dir().join(format!("{}.json", id));
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("读取会话文件失败: {}", path.display()))?;
    let file: SessionFile = serde_json::from_str(&content)
        .with_context(|| format!("解析会话文件失败: {}", path.display()))?;

    let messages: Vec<Message> = file.messages.iter().map(stored_to_message).collect();
    Ok(messages)
}

/// 加载最近一个会话
pub fn load_latest_session() -> Result<Option<Vec<Message>>> {
    let sessions = list_sessions()?;
    if let Some(latest) = sessions.first() {
        let messages = load_session(&latest.id)?;
        Ok(Some(messages))
    } else {
        Ok(None)
    }
}

// ────────────────────────────────────────────────────────────
// 内部转换
// ────────────────────────────────────────────────────────────

fn message_to_stored(msg: &Message) -> StoredMessage {
    StoredMessage {
        role: msg.role.as_str().to_string(),
        content: msg.content.clone(),
        tool_calls: msg.tool_calls.as_ref().map(|tcs| {
            tcs.iter()
                .map(|tc| StoredToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect()
        }),
        tool_call_id: msg.tool_call_id.clone(),
        name: msg.name.clone(),
    }
}

fn stored_to_message(stored: &StoredMessage) -> Message {
    let role = match stored.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "system" => Role::System,
        "tool" => Role::Tool,
        _ => Role::User, // fallback
    };

    let tool_calls = stored.tool_calls.as_ref().map(|tcs| {
        tcs.iter()
            .map(|tc| ToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect()
    });

    Message {
        role,
        content: stored.content.clone(),
        tool_calls,
        tool_call_id: stored.tool_call_id.clone(),
        name: stored.name.clone(),
    }
}

/// 生成会话 ID（基于当前时间）
fn generate_session_id() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", secs)
}

/// 当前时间戳字符串
fn current_timestamp() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 简单格式化（不引入 chrono）
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    format!("{}:{:02}:{:02}", secs / 86400, hours, mins)
}
