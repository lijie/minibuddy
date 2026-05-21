/// 配置系统：从 TOML 文件加载应用配置
///
/// 配置文件路径：~/.mini-buddy/config.toml
///
/// 优先级设计（高覆盖低）：
/// 1. 环境变量 LLM_PROVIDER → 覆盖 config 中的 default_provider
/// 2. 环境变量 DEEPSEEK_API_KEY 等 → 覆盖 config 中的 api_key
/// 3. 配置文件 → 持久化设置
/// 4. 内置默认值 → 兜底
///
/// 如果配置文件不存在，自动生成默认模板。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ────────────────────────────────────────────────────────────
// 配置结构体
// ────────────────────────────────────────────────────────────

/// 应用顶层配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 默认使用的 provider 名称（对应 providers 表中的 key）
    pub default_provider: String,
    /// 所有已配置的 provider
    pub providers: HashMap<String, ProviderConfig>,
    /// MCP 服务器配置（可选，Phase 8）
    #[serde(default)]
    pub mcp: Option<HashMap<String, McpServerConfig>>,
}

/// 单个 Provider 的配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider 类型："openai"（兼容 DeepSeek/Ollama 等）或 "anthropic"
    #[serde(rename = "type")]
    pub provider_type: String,
    /// API Key 来源的环境变量名（如 "DEEPSEEK_API_KEY"）
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// 直接写入的 API Key（不推荐，但在本地使用时方便）
    #[serde(default)]
    pub api_key: Option<String>,
    /// API 基础 URL（OpenAI 兼容格式必填，Anthropic 可选）
    #[serde(default)]
    pub base_url: Option<String>,
    /// 模型名称
    pub model: String,
    /// 最大输出 token 数（Anthropic 必填，OpenAI 可选）
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// MCP 服务器配置（Phase 8）
///
/// MCP (Model Context Protocol) 允许 LLM 通过标准化协议与外部工具交互。
/// 每个 MCP 服务器是一个独立的进程，通过 stdin/stdout 进行 JSON-RPC 通信。
///
/// 配置示例（TOML 格式）：
/// ```toml
/// [mcp.filesystem]
/// command = "mcp-filesystem"
/// args = ["/home/user/projects"]
///
/// [mcp.web-browser]
/// command = "/usr/local/bin/mcp-browser"
/// args = []
/// env = { BROWSER_TIMEOUT = "30" }
/// startup_timeout_secs = 10
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 可执行文件路径（绝对路径或在 PATH 中的命令）
    pub command: String,
    /// 命令行参数
    #[serde(default)]
    pub args: Vec<String>,
    /// 环境变量（可选，会与当前进程环境合并）
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// 可选：命令的工作目录（如果未指定，使用当前目录）
    #[serde(default)]
    pub cwd: Option<String>,
    /// 可选：启动超时时间（秒，默认 5 秒）
    #[serde(default)]
    pub startup_timeout_secs: Option<u64>,
}

// ────────────────────────────────────────────────────────────
// 配置加载
// ────────────────────────────────────────────────────────────

/// 获取配置文件路径：~/.mini-buddy/config.toml
pub fn config_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".mini-buddy").join("config.toml")
}

/// 加载配置
///
/// 加载优先级：
/// 1. 如果配置文件存在 → 读取并解析
/// 2. 如果不存在 → 生成默认模板并返回默认配置
/// 3. 环境变量在调用方处理（覆盖 config 中的值）
pub fn load_config() -> Result<Config> {
    let path = config_path();

    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("解析配置文件失败: {}", path.display()))?;
        Ok(config)
    } else {
        // 配置文件不存在，生成默认模板
        let config = default_config();
        save_config(&config)?;
        Ok(config)
    }
}

/// 保存配置到文件
pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path();

    // 创建目录
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    }

    let content = toml::to_string_pretty(config)
        .context("序列化配置失败")?;

    // 加上注释头（Phase 8 扩展：添加 MCP 文档）
    let with_header = format!(
        "# mini-buddy 配置文件\n\
         # 路径: {}\n\
         # 文档: https://github.com/xxx/mini-buddy#configuration\n\n\
         # Phase 6: 多 LLM Provider 支持 (deepseek, anthropic, ollama 等)\n\
         # Phase 8: MCP (Model Context Protocol) 服务器集成\n\
         #\n\
         # MCP 配置示例：\n\
         # [mcp.filesystem]\n\
         # command = \"mcp-filesystem\"\n\
         # args = [\"/home/user/projects\"]\n\
         # startup_timeout_secs = 10\n\
         #\n\
         # [mcp.web-browser]\n\
         # command = \"/usr/local/bin/mcp-browser\"\n\
         # env = {{ BROWSER_TIMEOUT = \"30\" }}\n\n\
         {}", path.display(), content
    );

    std::fs::write(&path, with_header)
        .with_context(|| format!("写入配置文件失败: {}", path.display()))?;

    Ok(())
}

// ────────────────────────────────────────────────────────────
// 默认配置
// ────────────────────────────────────────────────────────────

/// 生成内置默认配置
///
/// 包含三个预配置的 provider：deepseek、anthropic、ollama
/// 以及示例 MCP 服务器配置（注释形式，供用户启用）
fn default_config() -> Config {
    let mut providers = HashMap::new();

    providers.insert(
        "deepseek".to_string(),
        ProviderConfig {
            provider_type: "openai".to_string(),
            api_key_env: Some("DEEPSEEK_API_KEY".to_string()),
            api_key: None,
            base_url: Some("https://api.deepseek.com/v1".to_string()),
            model: "deepseek-chat".to_string(),
            max_tokens: None,
        },
    );

    providers.insert(
        "anthropic".to_string(),
        ProviderConfig {
            provider_type: "anthropic".to_string(),
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            api_key: None,
            base_url: None,
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: Some(4096),
        },
    );

    providers.insert(
        "ollama".to_string(),
        ProviderConfig {
            provider_type: "openai".to_string(),
            api_key_env: None,
            api_key: None,
            base_url: Some("http://localhost:11434/v1".to_string()),
            model: "qwen2.5".to_string(),
            max_tokens: None,
        },
    );

    // Phase 8：示例 MCP 服务器配置
    let mut mcp_servers = HashMap::new();

    // 示例：文件系统工具（来自 @anthropic/mcp-filesystem）
    mcp_servers.insert(
        "filesystem".to_string(),
        McpServerConfig {
            command: "mcp-filesystem".to_string(),
            args: vec![],
            env: None,
            cwd: None,
            startup_timeout_secs: Some(5),
        },
    );

    Config {
        default_provider: "deepseek".to_string(),
        providers,
        mcp: Some(mcp_servers),
    }
}

// ────────────────────────────────────────────────────────────
// 辅助方法
// ────────────────────────────────────────────────────────────

impl ProviderConfig {
    /// 解析 API Key：优先环境变量，其次配置文件中的 api_key 字段
    ///
    /// 为什么环境变量优先？
    /// - 安全：避免密钥写入文件被意外提交
    /// - 灵活：CI/CD 环境中用环境变量注入
    /// - 十二因素应用原则
    pub fn resolve_api_key(&self) -> Option<String> {
        // 1. 先查环境变量
        if let Some(ref env_name) = self.api_key_env {
            if let Ok(key) = std::env::var(env_name) {
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }
        // 2. 再查配置文件中直接写的 key
        self.api_key.clone()
    }

    /// 获取 base_url，如果未配置返回 provider 类型的默认值
    pub fn resolve_base_url(&self) -> String {
        if let Some(ref url) = self.base_url {
            url.clone()
        } else {
            match self.provider_type.as_str() {
                "anthropic" => "https://api.anthropic.com".to_string(),
                _ => "https://api.openai.com/v1".to_string(),
            }
        }
    }
}

impl Config {
    /// 获取当前生效的 provider 名称
    /// 环境变量 LLM_PROVIDER 覆盖配置文件中的 default_provider
    pub fn active_provider_name(&self) -> String {
        std::env::var("LLM_PROVIDER").unwrap_or_else(|_| self.default_provider.clone())
    }

    /// 获取当前生效的 provider 配置
    pub fn active_provider(&self) -> Result<&ProviderConfig> {
        let name = self.active_provider_name();
        self.providers
            .get(&name)
            .ok_or_else(|| anyhow::anyhow!(
                "配置中未找到 provider '{}'。已配置的: {:?}",
                name,
                self.providers.keys().collect::<Vec<_>>()
            ))
    }

    /// 获取 MCP 服务器配置（如果存在）
    /// Phase 8：新增方法用于访问 MCP 配置
    #[allow(dead_code)]
    pub fn mcp_servers(&self) -> Option<&HashMap<String, McpServerConfig>> {
        self.mcp.as_ref()
    }

    /// 检查是否启用了 MCP
    #[allow(dead_code)]
    pub fn has_mcp_enabled(&self) -> bool {
        self.mcp.as_ref().map(|m| !m.is_empty()).unwrap_or(false)
    }
}
