/// Anthropic Claude API 实现
///
/// 与 OpenAI 兼容格式的关键区别：
/// 1. system 消息不在 messages 数组中，是独立的顶层字段
/// 2. max_tokens 是必填字段（OpenAI 中可选）
/// 3. 认证用 x-api-key header，不是 Bearer token
/// 4. 流式事件格式完全不同（typed events，而非纯 data: 行）
///
/// 参考文档：https://docs.anthropic.com/en/api/messages

use anyhow::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;

use crate::llm::{LlmProvider, Message, Role, StreamChunk};

// ────────────────────────────────────────────────────────────
// Provider 结构体
// ────────────────────────────────────────────────────────────

pub struct AnthropicProvider {
    api_key: String,
    model: String,
    /// Anthropic API 的必填字段，限制最大输出 token 数
    max_tokens: u32,
    /// 复用 HTTP 连接
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String, max_tokens: u32) -> Self {
        Self {
            api_key,
            model,
            max_tokens,
            client: reqwest::Client::new(),
        }
    }

    /// 快捷构造：Claude Sonnet 默认配置
    pub fn claude_sonnet(api_key: String) -> Self {
        Self::new(api_key, "claude-sonnet-4-20250514".to_string(), 4096)
    }

    /// 将通用 Message 列表转换为 Anthropic 格式
    ///
    /// Anthropic 的特殊之处：system 消息不在 messages 数组中，
    /// 而是作为请求体的独立顶层字段。
    /// 返回 (system_prompt, user_assistant_messages)
    fn prepare_messages(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    // 如果有多条 system 消息，用最后一条（也可以拼接，这里选择简单方案）
                    system = Some(msg.content.clone());
                }
                Role::User => {
                    api_messages.push(serde_json::json!({
                        "role": "user",
                        "content": &msg.content,
                    }));
                }
                Role::Assistant => {
                    api_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": &msg.content,
                    }));
                }
            }
        }

        (system, api_messages)
    }

    /// 构建请求体
    fn build_request_body(
        &self,
        messages: &[Message],
        stream: bool,
    ) -> serde_json::Value {
        let (system, api_messages) = Self::prepare_messages(messages);

        let mut body = serde_json::json!({
            "model": &self.model,
            "max_tokens": self.max_tokens,
            "messages": api_messages,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::Value::String(sys);
        }

        if stream {
            body["stream"] = serde_json::Value::Bool(true);
        }

        body
    }
}

// ────────────────────────────────────────────────────────────
// LlmProvider 实现
// ────────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for AnthropicProvider {
    /// 非流式调用
    async fn chat(&self, messages: &[Message]) -> Result<String> {
        let body = self.build_request_body(messages, false);

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("发送请求失败: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API 错误 {}: {}", status, text);
        }

        // Anthropic 响应格式：
        // {"content": [{"type": "text", "text": "Hello!"}], "role": "assistant", ...}
        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("解析响应 JSON 失败: {}", e))?;

        let text = json["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(text)
    }

    /// 流式调用 — Anthropic SSE 事件解析
    ///
    /// Anthropic SSE 格式与 OpenAI 显著不同，使用 typed events：
    /// ```text
    /// event: message_start
    /// data: {"type":"message_start","message":{...}}
    ///
    /// event: content_block_start
    /// data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}
    ///
    /// event: content_block_delta
    /// data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}
    ///
    /// event: content_block_stop
    /// data: {"type":"content_block_stop","index":0}
    ///
    /// event: message_stop
    /// data: {"type":"message_stop"}
    /// ```
    ///
    /// 我们只关心 content_block_delta（文本内容）和 message_stop（结束信号）
    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + 'a>> {
        Box::pin(stream! {
            let body = self.build_request_body(messages, true);

            let response = self
                .client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    yield Err(anyhow::anyhow!("发送请求失败: {}", e));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                yield Err(anyhow::anyhow!("Anthropic API 错误 {}: {}", status, text));
                return;
            }

            // 逐块解析 SSE 事件
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow::anyhow!("读取响应流失败: {}", e));
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // 兼容 \r\n
                let normalized = buffer.replace("\r\n", "\n");
                buffer = normalized;

                // 按双换行分割完整事件
                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    // 解析事件类型和数据
                    // Anthropic 的 SSE 事件有 event: 和 data: 两行
                    let mut event_type = String::new();
                    let mut data_str = String::new();

                    for line in event_block.lines() {
                        if let Some(et) = line.strip_prefix("event: ") {
                            event_type = et.trim().to_string();
                        } else if let Some(d) = line.strip_prefix("data: ") {
                            data_str = d.to_string();
                        }
                    }

                    match event_type.as_str() {
                        "content_block_delta" => {
                            // 格式：{"type":"content_block_delta","delta":{"type":"text_delta","text":"..."}}
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data_str) {
                                if let Some(text) = json["delta"]["text"].as_str() {
                                    if !text.is_empty() {
                                        yield Ok(StreamChunk::Delta(text.to_string()));
                                    }
                                }
                            }
                        }
                        "message_stop" => {
                            yield Ok(StreamChunk::Done);
                            return;
                        }
                        // 忽略其他事件类型：
                        // - message_start: 消息开始，包含 message id、model 等元信息
                        // - content_block_start: 内容块开始
                        // - content_block_stop: 内容块结束
                        // - ping: 保活心跳
                        // Phase 10 可观测性阶段可以利用这些事件做 debug 输出
                        _ => {}
                    }
                }
            }

            // 异常结束兜底
            yield Ok(StreamChunk::Done);
        })
    }
}
