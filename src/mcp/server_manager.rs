/// MCP Server Manager
///
/// Manages the lifecycle of MCP server processes and provides
/// interfaces for tool discovery and invocation.

use super::types::*;
use super::transport::McpTransport;
use crate::config::McpServerConfig;
use anyhow::{Context, Result, bail};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Manages a single MCP server instance
///
/// This struct handles:
/// - Server process lifecycle (spawn, keep alive, cleanup)
/// - Tool discovery (listing all available tools)
/// - Tool invocation (executing tools with arguments)
/// - Error handling and connection recovery
pub struct McpServerManager {
    /// Name of the server (from config)
    pub name: String,
    
    /// Transport for JSON-RPC communication
    transport: Arc<Mutex<McpTransport>>,
    
    /// Cached tools (populated on first discovery)
    tools_cache: Arc<Mutex<Option<Vec<McpTool>>>>,
}

impl McpServerManager {
    /// Spawn a new MCP server and create a manager for it
    ///
    /// # Arguments
    /// * `name` - Server name (for logging/identification)
    /// * `config` - Server configuration from config file
    ///
    /// # Errors
    /// Returns error if process spawn fails
    pub async fn spawn(name: String, config: &McpServerConfig) -> Result<Self> {
        let transport = McpTransport::spawn(
            &config.command,
            config.args.clone(),
        ).await
            .context(format!("Failed to spawn MCP server '{}'", name))?;

        Ok(Self {
            name,
            transport: Arc::new(Mutex::new(transport)),
            tools_cache: Arc::new(Mutex::new(None)),
        })
    }

    /// List all available tools from this MCP server
    ///
    /// Caches the result after first call for performance.
    /// To refresh, create a new McpServerManager instance.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        // Check cache first
        {
            let cache = self.tools_cache.lock().await;
            if let Some(tools) = cache.as_ref() {
                return Ok(tools.clone());
            }
        }

        // Call RPC method if not cached
        let transport = self.transport.lock().await;
        let response = transport.call_method("tools/list", None).await?;

        // Parse response
        let tools_response: ToolsListResponse = serde_json::from_value(response)
            .context("Failed to parse tools/list response")?;

        // Cache the result
        let tools = tools_response.tools.clone();
        {
            let mut cache = self.tools_cache.lock().await;
            *cache = Some(tools_response.tools);
        }

        Ok(tools)
    }

    /// Call a tool on this MCP server
    ///
    /// # Arguments
    /// * `tool_name` - Name of the tool to invoke
    /// * `arguments` - Tool arguments as JSON value
    ///
    /// # Returns
    /// Result as a string (conversion from tool result content)
    pub async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<String> {
        let transport = self.transport.lock().await;
        
        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = transport.call_method("tools/call", Some(params)).await?;

        // Parse response
        let tool_response: ToolCallResponse = serde_json::from_value(response)
            .context("Failed to parse tools/call response")?;

        // Extract text result from response content
        if tool_response.isError {
            let error_msg = tool_response.content
                .first()
                .map(|c| c.to_string_representation())
                .unwrap_or_else(|| "Unknown error".to_string());
            bail!("Tool execution failed: {}", error_msg);
        }

        // Convert content to string
        let result = tool_response.content
            .iter()
            .map(|c| c.to_string_representation())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(result)
    }

    /// Get tool by name from available tools
    ///
    /// Returns None if tool not found
    pub async fn get_tool(&self, name: &str) -> Result<Option<McpTool>> {
        let tools = self.list_tools().await?;
        Ok(tools.into_iter().find(|t| t.name == name))
    }

    /// Get server name
    pub fn name(&self) -> &str {
        &self.name
    }
}

// ────────────────────────────────────────────────────────────
// Registry for managing multiple MCP servers
// ────────────────────────────────────────────────────────────

/// Manages multiple MCP server instances
///
/// Provides a centralized interface for working with multiple MCP servers.
pub struct McpServerRegistry {
    servers: Arc<Mutex<std::collections::HashMap<String, Arc<McpServerManager>>>>,
}

impl McpServerRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            servers: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Register an MCP server
    pub async fn register(&self, server: McpServerManager) -> Result<()> {
        let mut servers = self.servers.lock().await;
        let name = server.name().to_string();
        servers.insert(name, Arc::new(server));
        Ok(())
    }

    /// Get a registered server by name
    pub async fn get(&self, name: &str) -> Option<Arc<McpServerManager>> {
        let servers = self.servers.lock().await;
        servers.get(name).cloned()
    }

    /// List all registered server names
    pub async fn list_servers(&self) -> Vec<String> {
        let servers = self.servers.lock().await;
        servers.keys().cloned().collect()
    }

    /// Find which server has a particular tool
    pub async fn find_server_with_tool(&self, tool_name: &str) -> Option<(String, Arc<McpServerManager>)> {
        let servers = self.servers.lock().await;
        for (name, server) in servers.iter() {
            // Note: This is inefficient as written - a real implementation
            // might cache this information or use a lookup table
            if let Ok(tool) = server.get_tool(tool_name).await {
                if tool.is_some() {
                    return Some((name.clone(), server.clone()));
                }
            }
        }
        None
    }
}

impl Default for McpServerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_result_content_to_string() {
        let content = ToolResultContent::Text {
            text: "test output".to_string(),
        };
        assert_eq!(content.to_string_representation(), "test output");

        let content = ToolResultContent::Image {
            data: "base64data".to_string(),
            mimeType: "image/png".to_string(),
        };
        assert!(content.to_string_representation().contains("image/png"));
    }

    #[test]
    fn test_tool_call_response_parsing() {
        let json = r#"
        {
            "content": [
                {
                    "type": "text",
                    "text": "Command executed successfully"
                }
            ],
            "isError": false
        }
        "#;

        let response: ToolCallResponse = serde_json::from_str(json).unwrap();
        assert!(!response.isError);
        assert_eq!(response.content.len(), 1);
    }

    #[test]
    fn test_tools_list_response_parsing() {
        let json = r#"
        {
            "tools": [
                {
                    "name": "bash",
                    "description": "Execute bash commands",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "command": {"type": "string"}
                        }
                    }
                }
            ]
        }
        "#;

        let response: ToolsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.tools.len(), 1);
        assert_eq!(response.tools[0].name, "bash");
    }

    #[test]
    fn test_registry_creation() {
        let registry = McpServerRegistry::new();
        // Just verify it creates without error
        drop(registry);
    }
}
