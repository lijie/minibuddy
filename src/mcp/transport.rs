/// MCP Transport Layer
///
/// Handles JSON-RPC communication over stdin/stdout with MCP servers.
/// This layer manages:
/// - Process spawning and lifecycle
/// - Sending JSON-RPC requests
/// - Reading and parsing JSON-RPC responses
/// - Request/response matching via ID

use super::types::*;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Manages communication with a single MCP server
pub struct McpTransport {
    // Server process
    #[allow(dead_code)]
    process: Arc<Mutex<Child>>,
    
    // Pipes
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    
    // Request ID counter
    next_request_id: Arc<Mutex<u64>>,
}

impl McpTransport {
    /// Spawn a new MCP server process and create transport
    ///
    /// # Arguments
    /// * `command` - Executable path or command name
    /// * `args` - Command line arguments
    /// * `env` - Optional environment variables to set for the process
    /// * `cwd` - Optional working directory for the process
    ///
    /// # Errors
    /// Returns error if process spawn fails or pipes can't be set up
    pub async fn spawn(
        command: &str,
        args: Vec<String>,
        env: Option<&HashMap<String, String>>,
        cwd: Option<&str>,
    ) -> Result<Self> {
        use std::process::Stdio;

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // 设置环境变量（与当前进程环境合并）
        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }

        // 设置工作目录
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut process = cmd.spawn()
            .context(format!("Failed to spawn MCP server: {}", command))?;

        let stdin = process.stdin.take()
            .context("Failed to get stdin from MCP server")?;
        let stdout = process.stdout.take()
            .context("Failed to get stdout from MCP server")?;

        Ok(Self {
            process: Arc::new(Mutex::new(process)),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            next_request_id: Arc::new(Mutex::new(0)),
        })
    }

    /// Send a JSON-RPC request and wait for response
    ///
    /// This is the main interface for communicating with the MCP server.
    /// It handles:
    /// - Generating a unique request ID
    /// - Serializing the request to JSON
    /// - Sending over stdin
    /// - Reading and parsing the response
    /// - Matching response to request via ID
    pub async fn call_method(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let request_id = {
            let mut id_counter = self.next_request_id.lock().await;
            *id_counter += 1;
            *id_counter
        };

        let request = JsonRpcRequest::new(
            request_id,
            method.to_string(),
            params,
        );

        // Send request
        self.send_request(&request).await?;

        // Read response
        let response = self.read_response(request_id).await?;

        Ok(response)
    }

    /// Send a JSON-RPC request to the server
    async fn send_request(&self, request: &JsonRpcRequest) -> Result<()> {
        let json_str = serde_json::to_string(request)
            .context("Failed to serialize JSON-RPC request")?;

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(json_str.as_bytes()).await
            .context("Failed to write to MCP server stdin")?;
        stdin.write_all(b"\n").await
            .context("Failed to write newline")?;
        stdin.flush().await
            .context("Failed to flush stdin")?;

        Ok(())
    }

    /// Read a single response line from the server
    async fn read_response(&self, expected_id: u64) -> Result<Value> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();

        loop {
            line.clear();
            let n = stdout.read_line(&mut line).await
                .context("Failed to read from MCP server stdout")?;

            if n == 0 {
                bail!("MCP server closed connection unexpectedly");
            }

            let line = line.trim();
            if line.is_empty() {
                continue;  // Skip empty lines
            }

            // Try to parse as JSON-RPC response or error
            if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(line) {
                if response.id == expected_id {
                    return Ok(response.result);
                } else {
                    // Response ID doesn't match - this shouldn't happen in single-threaded usage
                    bail!("Response ID mismatch: expected {}, got {}", expected_id, response.id);
                }
            } else if let Ok(error) = serde_json::from_str::<JsonRpcError>(line) {
                if error.id == Some(expected_id) {
                    bail!(
                        "MCP server error (code {}): {}",
                        error.error.code,
                        error.error.message
                    );
                } else {
                    // Error ID doesn't match
                    bail!("Error response ID mismatch");
                }
            } else {
                // Not a valid JSON-RPC response - could be server log output
                // 用文件日志而非 eprintln!，避免 TUI raw mode 下的输出混乱
                crate::agent::log_info(&format!("MCP: Unexpected response line: {}", line));
                continue;
            }
        }
    }

    /// 关闭 MCP 服务器进程
    ///
    /// 尝试优雅关闭：先关闭 stdin（服务器应收到 EOF 后自行退出），
    /// 如果进程仍在运行则强制 kill。
    #[allow(dead_code)]
    pub async fn shutdown(&self) {
        // 关闭 stdin，通知服务器没有更多输入
        drop(self.stdin.lock().await);

        // 尝试 kill 进程
        let mut process = self.process.lock().await;
        let _ = process.kill().await;
    }
}

impl Drop for McpTransport {
    fn drop(&mut self) {
        // 尝试在 Drop 中 kill 进程
        // 由于 Drop 是同步的，无法 await，使用 tokio spawn 做 fire-and-forget cleanup
        let process = Arc::clone(&self.process);
        tokio::spawn(async move {
            let mut proc = process.lock().await;
            let _ = proc.kill().await;
        });
    }
}

// ────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_creation() {
        let req = JsonRpcRequest::new(
            1,
            "tools/list".to_string(),
            None,
        );

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"tools/list\""));
    }

    #[test]
    fn test_jsonrpc_response_parsing() {
        let response_json = r#"
        {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "tools": [
                    {
                        "name": "bash",
                        "description": "Execute bash commands",
                        "inputSchema": {"type": "object"}
                    }
                ]
            }
        }
        "#;

        let response: JsonRpcResponse = serde_json::from_str(response_json).unwrap();
        assert_eq!(response.id, 1);
        assert!(response.result["tools"].is_array());
    }

    #[test]
    fn test_jsonrpc_error_parsing() {
        let error_json = r#"
        {
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32600,
                "message": "Invalid Request"
            }
        }
        "#;

        let error: JsonRpcError = serde_json::from_str(error_json).unwrap();
        assert_eq!(error.error.code, -32600);
        assert_eq!(error.error.message, "Invalid Request");
    }

    #[test]
    fn test_request_id_generation() {
        // This test doesn't require async runtime for basic serialization testing
        let req1 = JsonRpcRequest::new(1, "test".to_string(), None);
        let req2 = JsonRpcRequest::new(2, "test".to_string(), None);

        assert_eq!(req1.id, 1);
        assert_eq!(req2.id, 2);
    }
}
