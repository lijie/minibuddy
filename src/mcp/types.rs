/// MCP (Model Context Protocol) type definitions
///
/// This module defines the JSON-RPC message types used to communicate
/// with MCP servers over stdin/stdout.
///
/// Reference: https://modelcontextprotocol.io/

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ────────────────────────────────────────────────────────────
// JSON-RPC Message Types
// ────────────────────────────────────────────────────────────

/// A JSON-RPC request sent to the MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,        // Always "2.0"
    pub id: u64,                // Request ID for matching responses
    pub method: String,         // RPC method name (e.g., "tools/list")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,  // Method parameters
}

/// A successful JSON-RPC response from the MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,        // Always "2.0"
    pub id: u64,                // Request ID this is responding to
    pub result: Value,          // Response data
}

/// A JSON-RPC error response from the MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub jsonrpc: String,        // Always "2.0"
    pub id: Option<u64>,        // Request ID (None for parse errors)
    pub error: ErrorObject,     // Error details
}

/// Error details in JSON-RPC error response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,              // Error code
    pub message: String,        // Error message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,    // Additional error data
}

// ────────────────────────────────────────────────────────────
// MCP Tool Types
// ────────────────────────────────────────────────────────────

/// Response from tools/list RPC call
/// Lists all available tools on the MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsListResponse {
    pub tools: Vec<McpTool>,
}

/// A single tool definition from MCP server
/// This is what we'll wrap with our Tool trait
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,  // JSON Schema for tool input
}

/// Response from tools/call RPC call
/// Contains the result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    #[serde(default)]
    pub content: Vec<ToolResultContent>,
    #[serde(default, rename = "isError")]
    pub is_error: bool,  // true if tool execution failed
}

/// Content within a tool call response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolResultContent {
    #[serde(rename = "text")]
    Text { text: String },
    
    #[serde(rename = "image")]
    Image {
        data: String,           // Base64-encoded image data
        #[serde(rename = "mimeType")]
        mime_type: String,       // e.g., "image/png"
    },

    #[serde(rename = "resource")]
    Resource {
        uri: String,            // Resource URI
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(default)]
        text: Option<String>,   // Optional text representation
    },
}

// ────────────────────────────────────────────────────────────
// Helper Functions
// ────────────────────────────────────────────────────────────

impl JsonRpcRequest {
    /// Create a new JSON-RPC request
    pub fn new(id: u64, method: String, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method,
            params,
        }
    }
}

impl ErrorObject {
    /// Create a parse error
    #[allow(dead_code)]
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: message.into(),
            data: None,
        }
    }

    /// Create an invalid request error
    #[allow(dead_code)]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    /// Create a method not found error
    #[allow(dead_code)]
    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: message.into(),
            data: None,
        }
    }

    /// Create an internal error
    #[allow(dead_code)]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }

    /// Create a server error (codes -32768 to -32000)
    #[allow(dead_code)]
    pub fn server_error(code: i32, message: impl Into<String>) -> Self {
        let code = if code >= -32768 && code <= -32000 {
            code
        } else {
            -32000
        };
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl ToolResultContent {
    /// Extract text from tool result content
    /// Returns the first text content found, or None if no text content exists
    #[allow(dead_code)]
    pub fn as_text(&self) -> Option<String> {
        match self {
            Self::Text { text } => Some(text.clone()),
            Self::Image { .. } => None,
            Self::Resource { text, .. } => text.clone(),
        }
    }

    /// Convert all content to a combined string representation
    pub fn to_string_representation(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Image { data, mime_type } => {
                format!("[Image: {}, size: {} bytes]", mime_type, data.len())
            }
            Self::Resource { uri, mime_type, text } => {
                if let Some(t) = text {
                    format!("[Resource: {} ({}): {}]", uri, mime_type, t)
                } else {
                    format!("[Resource: {} ({})]", uri, mime_type)
                }
            }
        }
    }
}

// ────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_serialization() {
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
    fn test_jsonrpc_request_with_params() {
        let params = serde_json::json!({
            "name": "bash",
            "arguments": {"command": "ls"}
        });

        let req = JsonRpcRequest::new(
            2,
            "tools/call".to_string(),
            Some(params.clone()),
        );

        let json = serde_json::to_string(&req).unwrap();
        let parsed: JsonRpcRequest = serde_json::from_str(&json).unwrap();
        
        assert_eq!(parsed.id, 2);
        assert_eq!(parsed.method, "tools/call");
        assert!(parsed.params.is_some());
    }

    #[test]
    fn test_mcp_tool_deserialization() {
        let json = r#"
        {
            "name": "bash",
            "description": "Execute a bash command",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to run"
                    }
                },
                "required": ["command"]
            }
        }
        "#;

        let tool: McpTool = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.description, "Execute a bash command");

        // Verify input_schema is valid JSON Schema
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema["properties"]["command"].is_object());
    }

    #[test]
    fn test_tool_result_content_text() {
        let content = ToolResultContent::Text {
            text: "test output".to_string(),
        };

        assert_eq!(content.as_text(), Some("test output".to_string()));
        assert_eq!(content.to_string_representation(), "test output");
    }

    #[test]
    fn test_error_object_codes() {
        let parse_err = ErrorObject::parse_error("Invalid JSON");
        assert_eq!(parse_err.code, -32700);

        let method_err = ErrorObject::method_not_found("Unknown method");
        assert_eq!(method_err.code, -32601);

        let internal_err = ErrorObject::internal_error("Server crashed");
        assert_eq!(internal_err.code, -32603);
    }
}
