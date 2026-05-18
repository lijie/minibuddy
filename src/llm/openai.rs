/// OpenAI 兼容接口实现
///
/// 覆盖范围：DeepSeek / Qwen / Kimi / 月之暗面 / Groq 等所有兼容 OpenAI Chat Completions 格式的服务。
/// 使用方法：只需修改 base_url 和 model，其余逻辑完全复用。
///
/// Phase 2 变更：新增 chat_stream() 方法，实现 SSE 流式输出
/// Phase 3 变更：新增 chat_with_tools() 方法，支持工具调用

use anyhow::{Context, Result};
use async_stream::stream;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

use crate::llm::{LlmProvider, LlmResponse, Message, Role, StreamChunk, ToolCall, ToolDefinition};

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

    // ── Phase 3: 工具调用辅助方法 ──────────────────────────────

    /// 构建带工具定义的请求体
    ///
    /// OpenAI 工具调用 API 格式：
    /// - tools 数组：[{type: "function", function: {name, description, parameters}}]
    /// - 消息格式需要处理三种新情况：
    ///   1. 普通消息：{role, content}（不变）
    ///   2. 带工具调用的 assistant 消息：{role: "assistant", tool_calls: [...]}
    ///   3. 工具结果消息：{role: "tool", tool_call_id: "...", content: "..."}
    fn build_tool_request_body(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Value {
        // 转换消息列表，处理不同角色的格式差异
        let api_messages: Vec<Value> = messages
            .iter()
            .map(|msg| match msg.role {
                // 工具结果消息：需要 tool_call_id 来关联回对应的工具调用
                Role::Tool => {
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                        "content": &msg.content,
                    })
                }
                // 带工具调用的 assistant 消息：重放历史时 API 要求包含原始的 tool_calls
                Role::Assistant if msg.tool_calls.is_some() => {
                    let tool_calls: Vec<Value> = msg
                        .tool_calls
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    // OpenAI 格式要求 arguments 是 JSON 字符串
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();

                    let mut obj = serde_json::json!({
                        "role": "assistant",
                        "tool_calls": tool_calls,
                    });
                    // content 字段：有内容则加上，否则设为 null
                    if !msg.content.is_empty() {
                        obj["content"] = Value::String(msg.content.clone());
                    }
                    obj
                }
                // 普通消息：user / assistant / system
                _ => {
                    serde_json::json!({
                        "role": msg.role.as_str(),
                        "content": &msg.content,
                    })
                }
            })
            .collect();

        // 转换工具定义为 OpenAI 格式
        let api_tools: Vec<Value> = tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();

        serde_json::json!({
            "model": &self.model,
            "messages": api_messages,
            "tools": api_tools,
        })
    }

    /// 解析 OpenAI 工具调用响应
    ///
    /// OpenAI 响应格式：
    /// {
    ///   "choices": [{
    ///     "message": {
    ///       "content": "可选的文本",
    ///       "tool_calls": [{
    ///         "id": "call_xxx",
    ///         "type": "function",
    ///         "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}
    ///       }]
    ///     },
    ///     "finish_reason": "tool_calls" | "stop"
    ///   }]
    /// }
    fn parse_tool_response(&self, response_text: &str) -> Result<LlmResponse> {
        let json: Value = serde_json::from_str(response_text)
            .context("解析工具调用响应 JSON 失败")?;

        let choice = json["choices"]
            .get(0)
            .ok_or_else(|| anyhow::anyhow!("响应中没有 choices"))?;

        let message = &choice["message"];

        // 提取文本内容（纯工具调用时可能为 null）
        let content = message["content"]
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());

        // 提取工具调用列表
        let tool_calls = if let Some(calls) = message["tool_calls"].as_array() {
            calls
                .iter()
                .map(|call| {
                    let id = call["id"].as_str().unwrap_or("").to_string();
                    let name = call["function"]["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    // OpenAI 返回的 arguments 是 JSON 字符串，需要解析为 Value
                    // 为什么是字符串？这是 OpenAI API 的设计决定
                    let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments: Value = serde_json::from_str(args_str)
                        .unwrap_or(Value::Object(serde_json::Map::new()));

                    ToolCall {
                        id,
                        name,
                        arguments,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(LlmResponse {
            content,
            tool_calls,
        })
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
    /// 非流式调用（Phase 1 逻辑，保持不变）
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

    /// 流式调用 — SSE (Server-Sent Events) 解析
    ///
    /// OpenAI SSE 协议格式：
    /// ```text
    /// data: {"choices":[{"delta":{"content":"你"}}]}
    ///
    /// data: {"choices":[{"delta":{"content":"好"}}]}
    ///
    /// data: [DONE]
    /// ```
    ///
    /// 为什么手动解析而不用 eventsource 库？
    /// - 教学目的：SSE 协议极简（data: 前缀 + 双换行分隔），手动解析让学习者理解协议本质
    /// - SSE 本质就是按行读取文本流，不需要复杂库
    fn chat_stream<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send + 'a>> {
        Box::pin(stream! {
            // 1. 构建请求体 — 与 chat() 相同，多加 "stream": true
            //    使用 serde_json::json! 宏是因为需要动态加 stream 字段
            //    不复用 ChatRequest 结构体，因为它没有 stream 字段
            let api_messages: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| serde_json::json!({
                    "role": m.role.as_str(),
                    "content": &m.content,
                }))
                .collect();

            let body = serde_json::json!({
                "model": &self.model,
                "messages": api_messages,
                "stream": true,
            });

            let url = format!("{}/chat/completions", self.base_url);

            // 2. 发送请求，获取字节流
            let response = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
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

            // 检查 HTTP 状态码
            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                yield Err(anyhow::anyhow!("API 返回错误 {}: {}", status, text));
                return;
            }

            // 3. 逐块读取响应体，解析 SSE 事件
            //
            // SSE 协议要点：
            // - 每个事件以双换行 \n\n 分隔
            // - 数据行以 "data: " 为前缀
            // - 流结束标记为 "data: [DONE]"
            //
            // 注意：HTTP 响应可能在任意位置切断字节流（甚至在 UTF-8 多字节字符中间），
            // 所以需要用 buffer 累积，按 \n\n 分割完整事件
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

                // 将字节追加到缓冲区
                // 注意：from_utf8_lossy 对不完整的 UTF-8 序列会产生 replacement character (U+FFFD)
                // 实际上 DeepSeek 等 API 通常按 token 边界发送，极少切断 UTF-8
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // 兼容性处理：某些代理服务器用 \r\n 代替 \n
                let normalized = buffer.replace("\r\n", "\n");
                buffer = normalized;

                // 按双换行分割事件
                while let Some(pos) = buffer.find("\n\n") {
                    let event = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    // 解析事件中的每一行
                    for line in event.lines() {
                        if let Some(data) = line.strip_prefix("data: ") {
                            let data = data.trim();

                            // 流结束标记
                            if data == "[DONE]" {
                                yield Ok(StreamChunk::Done);
                                return;
                            }

                            // 解析 JSON，提取增量内容
                            // 格式：{"choices":[{"delta":{"content":"token"}}]}
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(content) =
                                    json["choices"][0]["delta"]["content"].as_str()
                                {
                                    if !content.is_empty() {
                                        yield Ok(StreamChunk::Delta(content.to_string()));
                                    }
                                }
                                // delta 可能只有 role 字段（第一个 chunk），或为空（某些 API）
                                // 这些情况静默跳过即可
                            }
                        }
                        // 忽略非 data: 开头的行（如 event:、id:、retry: 等 SSE 字段）
                    }
                }
            }

            // 如果流正常结束但没收到 [DONE]（某些 API 实现不规范），也发送结束信号
            yield Ok(StreamChunk::Done);
        })
    }

    /// Phase 3: 带工具定义的非流式对话
    ///
    /// 工作流程：
    /// 1. 将 messages + tools 转换为 OpenAI API 格式
    /// 2. 发送 HTTP 请求
    /// 3. 解析响应中的 content 和 tool_calls
    /// 4. 返回统一的 LlmResponse
    async fn chat_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let body = self.build_tool_request_body(messages, tools);

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("发送工具调用请求失败，请检查网络连接")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("API 返回错误 {}: {}", status, error_text);
        }

        let response_text = response
            .text()
            .await
            .context("读取响应文本失败")?;

        self.parse_tool_response(&response_text)
    }
}
