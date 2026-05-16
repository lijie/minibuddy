/// OpenAI 兼容接口实现
///
/// 覆盖范围：DeepSeek / Qwen / Kimi / 月之暗面 / Groq 等所有兼容 OpenAI Chat Completions 格式的服务。
/// 使用方法：只需修改 base_url 和 model，其余逻辑完全复用。

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::llm::{LlmProvider, Message};

// ────────────────────────────────────────────────────────────
// Provider 结构体
// ────────────────────────────────────────────────────────────

pub struct OpenAIProvider {
    api_key: String,
    /// API 基础地址，不含路径，例如 "https://api.deepseek.com/v1"
    base_url: String,
    /// 模型名称，例如 "deepseek-chat"
    model: String,
    /// 复用 HTTP 连接（reqwest::Client 内部维护连接池）
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        Self {
            api_key,
            base_url,
            model,
            // 设计原因：Client 应该被复用，每次请求都 new() 会丢失连接池和 TLS session
            client: reqwest::Client::new(),
        }
    }

    /// 快捷构造：DeepSeek 默认配置
    pub fn deepseek(api_key: String) -> Self {
        Self::new(
            api_key,
            "https://api.deepseek.com/v1".to_string(),
            "deepseek-chat".to_string(),
        )
    }
}

// ────────────────────────────────────────────────────────────
// 请求 / 响应数据结构（对应 OpenAI Chat Completions API）
// ────────────────────────────────────────────────────────────

/// POST /v1/chat/completions 请求体
#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
}

/// 请求中的单条消息
#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// 响应体（只取需要的字段，其余忽略）
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: String,
}

// ────────────────────────────────────────────────────────────
// LlmProvider 实现
// ────────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OpenAIProvider {
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        // 将内部 Message 类型转换为 API 请求格式
        let api_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.as_str(),
                content: &m.content,
            })
            .collect();

        let request_body = ChatRequest {
            model: &self.model,
            messages: api_messages,
        };

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            // Bearer token 认证（OpenAI / DeepSeek 等均使用此方式）
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .await
            .context("发送 HTTP 请求失败，请检查网络连接")?;

        // 先检查 HTTP 状态码，非 2xx 时提取错误信息
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("API 返回错误 {status}: {error_text}");
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .context("解析响应 JSON 失败")?;

        // 取第一个 choice 的 content
        let content = chat_response
            .choices
            .into_iter()
            .next()
            .context("API 响应中没有 choices")?
            .message
            .content;

        Ok(content)
    }
}
