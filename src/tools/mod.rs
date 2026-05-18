/// 工具系统：让 Agent 能够与外部世界交互
///
/// 设计理念：
/// - Tool trait 定义统一接口，方便扩展新工具（Phase 8 的 MCP 工具也实现此 trait）
/// - ToolRegistry 集中管理所有可用工具，Agent Loop 通过它来查找和执行工具
/// - 工具的 parameters_schema() 返回 JSON Schema，LLM 据此生成合法参数
///
/// 架构位置：
/// ```
/// Agent Loop → ToolRegistry.get(name) → Tool.execute(args) → 结果文本
///                    ↓
///            ToolRegistry.definitions() → 传给 LLM Provider
/// ```

pub mod bash;
pub mod read_file;
pub mod sandbox;       // Phase 4: 命令安全分级
pub mod write_file;    // Phase 4: 文件写入工具

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

use crate::llm::ToolDefinition;

// ────────────────────────────────────────────────────────────
// Tool trait
// ────────────────────────────────────────────────────────────

/// 工具 trait：所有工具必须实现此接口
///
/// 为什么用 trait 而不是 enum？
/// - 开放扩展：Phase 8 的 MCP 工具、用户自定义工具都可以实现此 trait
/// - 关注点分离：每个工具的逻辑独立封装在自己的文件中
/// - 动态注册：ToolRegistry 持有 Box<dyn Tool>，运行时可添加新工具
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名称（唯一标识符，LLM 用这个名字来调用）
    fn name(&self) -> &str;

    /// 工具描述（自然语言，帮助 LLM 理解何时使用此工具）
    /// 描述质量直接影响 LLM 的工具选择准确性
    fn description(&self) -> &str;

    /// 参数的 JSON Schema（告诉 LLM 需要传什么参数、什么类型）
    ///
    /// 返回 Value 而非强类型，因为 JSON Schema 本身结构灵活，
    /// 且不同工具的参数完全不同
    fn parameters_schema(&self) -> Value;

    /// 执行工具，返回结果文本
    ///
    /// 为什么返回 String 而不是 Value？
    /// - LLM 需要阅读结果来做后续推理，纯文本最通用
    /// - 文件内容、命令输出、错误信息本来就是文本
    /// - 保持简单：工具只需要关心"产出什么文本给 LLM 看"
    async fn execute(&self, arguments: Value) -> Result<String>;
}

// ────────────────────────────────────────────────────────────
// ToolRegistry
// ────────────────────────────────────────────────────────────

/// 工具注册表：集中管理所有可用工具
///
/// 职责：
/// 1. 存储工具实例（HashMap<name, Box<dyn Tool>>）
/// 2. 按名字查找工具（Agent Loop 执行时调用）
/// 3. 生成 ToolDefinition 列表（传给 LLM Provider，让 LLM 知道有哪些工具可用）
///
/// 为什么单独做一个 Registry 而不是直接 Vec<Box<dyn Tool>>？
/// - 按名字查找是 O(1)（HashMap），Vec 需要遍历
/// - 未来可以在这里加权限检查、调用频率限制等
/// - 清晰的职责边界：Agent 不直接持有工具，通过 Registry 间接访问
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// 注册一个工具（如果名字冲突，后注册的会覆盖先注册的）
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// 按名字查找工具
    /// 返回 Option 是因为 LLM 可能"幻觉"一个不存在的工具名
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// 列出所有已注册工具的名字（用于错误提示）
    pub fn list_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// 生成所有工具的定义列表
    /// 这是 Tool trait 和 LLM Provider 之间的桥梁——
    /// Tool 实现者提供 name/description/schema，
    /// 这里打包成 ToolDefinition 传给 Provider
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters_schema(),
            })
            .collect()
    }
}

// ────────────────────────────────────────────────────────────
// 默认工具注册
// ────────────────────────────────────────────────────────────

/// 创建包含所有内置工具的注册表
///
/// Phase 3 内置工具：bash + read_file
/// Phase 4 新增：write_file（带确认流程）
/// Phase 8 将新增 MCP 动态工具
pub fn create_default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(bash::BashTool));
    registry.register(Box::new(read_file::ReadFileTool));
    registry.register(Box::new(write_file::WriteFileTool));
    registry
}
