/// Anthropic Claude API 实现
///
/// 与 OpenAI 兼容格式的关键区别：
/// 1. system 消息不在 messages 数组中，是独立的顶层字段
/// 2. max_tokens 是必填字段（OpenAI 中可选）
/// 3. 认证用 x-api-key header，不是 Bearer token
/// 4. 流式事件格式完全不同（typed events，而非纯 data: 行）
///
/// Phase 3 新增区别（工具调用格式）：
/// 5. 工具定义用 input_schema（OpenAI 用 parameters）
/// 6. 工具调用在 content blocks 中（type: "tool_use"），不是独立的 tool_calls 字段
/// 7. 工具结果是 user 消息的 content block（type: "tool_result"），不是 role: "tool"
/// 8. 连续的 tool_result 必须合并到同一个 user 消息（不允许连续同角色消息）
///
/// 参考文档：https://docs.anthropic.com/en/api/messages

use anyhow::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::Value;
use std::pin::Pin;

use crate::llm::{LlmProvider, LlmResponse, Message, Role, StreamChunk, ToolCall, ToolDefinition};

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
                Role::Tool => {
                    // Phase 3: prepare_messages 只用于非工具模式（chat/chat_stream），
                    // 工具模式走 build_tool_request_body。这里跳过 Tool 消息。
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

    // ── Phase 3: 工具调用辅助方法 ──────────────────────────────

    /// 构建带工具定义的请求体（Anthropic 格式）
    ///
    /// 与 build_request_body 的区别：
    /// 1. 多了 tools 数组
    /// 2. 消息转换更复杂（需处理 tool_use / tool_result content blocks）
    /// 3. 连续的 tool_result 消息需要合并为单个 user 消息
    fn build_tool_request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Value {
        // 提取 system 消息（Anthropic 放在顶层，不在 messages 数组中）
        let system_content = messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone());

        // 转换消息列表（跳过 system，处理工具相关的特殊格式）
        let mut api_messages: Vec<Value> = Vec::new();

        for msg in messages.iter().filter(|m| m.role != Role::System) {
            let converted = self.convert_message_for_tools(msg);

            // Anthropic API 不允许连续的同角色消息
            // 当一次 LLM 调用返回多个工具调用时，每个工具结果都是 Role::Tool，
            // 转换后都变成 role: "user"，需要合并到同一个 user 消息中
            if msg.role == Role::Tool {
                if let Some(last) = api_messages.last_mut() {
                    if last["role"] == "user" {
                        // 将新的 content blocks 合并到已有的 user 消息中
                        if let (Some(last_content), Some(new_content)) = (
                            last["content"].as_array_mut(),
                            converted["content"].as_array(),
                        ) {
                            last_content.extend(new_content.iter().cloned());
                            continue; // 已合并，不需要 push
                        }
                    }
                }
            }

            api_messages.push(converted);
        }

        // 转换工具定义（Anthropic 用 input_schema 而非 parameters）
        let api_tools: Vec<Value> = tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": &self.model,
            "max_tokens": self.max_tokens,
            "messages": api_messages,
            "tools": api_tools,
        });

        if let Some(sys) = system_content {
            body["system"] = Value::String(sys);
        }

        body
    }

    /// 将单条 Message 转换为 Anthropic API 格式
    ///
    /// Anthropic 的消息转换比 OpenAI 复杂：
    /// - 普通消息：{role, content: "text"}
    /// - 带工具调用的 assistant：{role: "assistant", content: [{type: "text"}, {type: "tool_use"}]}
    /// - 工具结果：{role: "user", content: [{type: "tool_result", tool_use_id, content}]}
    fn convert_message_for_tools(&self, msg: &Message) -> Value {
        match msg.role {
            // 工具结果：Anthropic 将其作为 user 消息的 content block
            // （与 OpenAI 的 role: "tool" 不同）
            Role::Tool => {
                serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                        "content": &msg.content,
                    }]
                })
            }
            // 带工具调用的 assistant 消息：用 content blocks 表示
            Role::Assistant if msg.tool_calls.is_some() => {
                let mut content_blocks: Vec<Value> = Vec::new();

                // 先加文本块（如果有思考内容）
                if !msg.content.is_empty() {
                    content_blocks.push(serde_json::json!({
                        "type": "text",
                        "text": &msg.content,
                    }));
                }

                // 再加工具调用块
                for tc in msg.tool_calls.as_ref().unwrap() {
                    content_blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        // Anthropic 的 input 直接用 JSON 对象，不需要字符串化
                        "input": tc.arguments,
                    }));
                }

                serde_json::json!({
                    "role": "assistant",
                    "content": content_blocks,
                })
            }
            // 普通 user / assistant 消息
            _ => {
                serde_json::json!({
                    "role": msg.role.as_str(),
                    "content": &msg.content,
                })
            }
        }
    }

    /// 解析 Anthropic 工具调用响应
    ///
    /// Anthropic 响应格式：
    /// {
    ///   "content": [
    ///     {"type": "text", "text": "让我查看..."},
    ///     {"type": "tool_use", "id": "toolu_xxx", "name": "bash", "input": {"command": "ls"}}
    ///   ],
    ///   "stop_reason": "tool_use" | "end_turn"
    /// }
    ///
    /// 与 OpenAI 的关键区别：
    /// - 文本和工具调用都在 content blocks 数组中混合存在
    /// - input 直接是 JSON 对象（不是字符串）
    fn parse_tool_response(&self, response_text: &str) -> Result<LlmResponse> {
        let json: Value = serde_json::from_str(response_text)
            .map_err(|e| anyhow::anyhow!("解析工具调用响应 JSON 失败: {}", e))?;

        let content_blocks = json["content"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("响应中没有 content 数组"))?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for block in content_blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str() {
                        if !text.is_empty() {
                            text_parts.push(text.to_string());
                        }
                    }
                }
                Some("tool_use") => {
                    tool_calls.push(ToolCall {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        name: block["name"].as_str().unwrap_or("").to_string(),
                        // Anthropic 直接返回 JSON 对象，无需额外解析
                        arguments: block["input"].clone(),
                    });
                }
                _ => {} // 忽略未知类型（前向兼容）
            }
        }

        // 合并所有文本块
        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        Ok(LlmResponse {
            content,
            tool_calls,
        })
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

    /// Phase 3: 带工具定义的非流式对话（Anthropic 格式）
    ///
    /// 工作流程与 OpenAI 类似，但 API 格式差异很大：
    /// - 工具定义用 input_schema
    /// - 响应中 content blocks 混合了文本和工具调用
    /// - stop_reason 为 "tool_use" 表示需要调用工具
    async fn chat_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let body = self.build_tool_request_body(messages, tools);

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("发送工具调用请求失败: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API 错误 {}: {}", status, text);
        }

        let response_text = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("读取响应文本失败: {}", e))?;

        self.parse_tool_response(&response_text)
    }
}
