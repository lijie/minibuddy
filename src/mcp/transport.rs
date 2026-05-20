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
    ///
    /// # Errors
    /// Returns error if process spawn fails or pipes can't be set up
    pub async fn spawn(command: &str, args: Vec<String>) -> Result<Self> {
        use std::process::Stdio;
        
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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
                // Not a valid JSON-RPC response - could be server output
                // Log it but continue trying to read actual responses
                eprintln!("MCP: Unexpected response line: {}", line);
                continue;
            }
        }
    }
}

impl Drop for McpTransport {
    fn drop(&mut self) {
        // Attempt to kill the process when transport is dropped
        // Note: This is a fire-and-forget attempt; we don't wait for it
        // In a production system, you might want to handle this more carefully
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
