/// Integration tests for Phase 8: Config system with MCP support

use std::collections::HashMap;

// We need to copy the structures here since they're in the main binary
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub default_provider: String,
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub mcp: Option<HashMap<String, McpServerConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub startup_timeout_secs: Option<u64>,
}

#[test]
fn test_config_with_mcp_deserialization() {
    let toml_str = r#"
default_provider = "deepseek"

[providers.deepseek]
type = "openai"
model = "deepseek-chat"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"

[mcp.filesystem]
command = "mcp-filesystem"
args = []
startup_timeout_secs = 5
"#;

    let config: Config = toml::from_str(toml_str).expect("Failed to parse TOML with MCP");
    
    assert_eq!(config.default_provider, "deepseek");
    assert_eq!(config.providers.len(), 1);
    
    let mcp = config.mcp.expect("MCP should be present");
    assert_eq!(mcp.len(), 1);
    
    let filesystem_server = mcp.get("filesystem").expect("filesystem server should exist");
    assert_eq!(filesystem_server.command, "mcp-filesystem");
    assert_eq!(filesystem_server.startup_timeout_secs, Some(5));
}

#[test]
fn test_config_without_mcp_backward_compat() {
    let toml_str = r#"
default_provider = "anthropic"

[providers.anthropic]
type = "anthropic"
model = "claude-sonnet"
"#;

    let config: Config = toml::from_str(toml_str).expect("Failed to parse TOML without MCP");
    
    assert_eq!(config.default_provider, "anthropic");
    assert_eq!(config.providers.len(), 1);
    assert!(config.mcp.is_none(), "MCP should be None for old configs");
}

#[test]
fn test_config_multiple_mcp_servers() {
    let toml_str = r#"
default_provider = "deepseek"

[providers.deepseek]
type = "openai"
model = "deepseek-chat"

[mcp.filesystem]
command = "mcp-filesystem"

[mcp.web-browser]
command = "mcp-browser"
args = ["--headless"]

[mcp.database]
command = "mcp-db"
env = { DB_HOST = "localhost", DB_PORT = "5432" }
cwd = "/opt/db"
startup_timeout_secs = 10
"#;

    let config: Config = toml::from_str(toml_str).expect("Failed to parse TOML with multiple MCP");
    
    let mcp = config.mcp.expect("MCP should be present");
    assert_eq!(mcp.len(), 3);
    
    // Check filesystem server
    let fs_server = mcp.get("filesystem").expect("filesystem should exist");
    assert_eq!(fs_server.command, "mcp-filesystem");
    assert!(fs_server.env.is_none());
    
    // Check web-browser server
    let browser_server = mcp.get("web-browser").expect("web-browser should exist");
    assert_eq!(browser_server.args.len(), 1);
    assert_eq!(browser_server.args[0], "--headless");
    
    // Check database server
    let db_server = mcp.get("database").expect("database should exist");
    assert_eq!(db_server.env.as_ref().unwrap().get("DB_HOST"), Some(&"localhost".to_string()));
    assert_eq!(db_server.cwd, Some("/opt/db".to_string()));
    assert_eq!(db_server.startup_timeout_secs, Some(10));
}

#[test]
fn test_config_serialization_roundtrip() {
    let toml_str = r#"
default_provider = "deepseek"

[providers.deepseek]
type = "openai"
model = "deepseek-chat"

[mcp.filesystem]
command = "mcp-filesystem"
startup_timeout_secs = 5
"#;

    let config: Config = toml::from_str(toml_str).expect("Failed to parse");
    let serialized = toml::to_string_pretty(&config).expect("Failed to serialize");
    let config2: Config = toml::from_str(&serialized).expect("Failed to parse serialized");
    
    assert_eq!(config.default_provider, config2.default_provider);
    assert_eq!(config.providers.len(), config2.providers.len());
    assert_eq!(config.mcp.is_some(), config2.mcp.is_some());
}

#[test]
fn test_mcp_empty_args_default() {
    let toml_str = r#"
default_provider = "deepseek"

[providers.deepseek]
type = "openai"
model = "deepseek-chat"

[mcp.test-server]
command = "test-command"
"#;

    let config: Config = toml::from_str(toml_str).expect("Failed to parse");
    let mcp = config.mcp.expect("MCP should be present");
    let server = mcp.get("test-server").expect("test-server should exist");
    
    assert_eq!(server.args.len(), 0);
    assert!(server.env.is_none());
    assert!(server.cwd.is_none());
    assert!(server.startup_timeout_secs.is_none());
}
