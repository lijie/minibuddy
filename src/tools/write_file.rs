/// 文件写入工具：安全地将内容写入文件
///
/// 为什么需要单独的 write_file 工具而不是让 LLM 用 bash 的 echo > file？
/// 1. 语义明确：LLM 调用 write_file 时意图清晰，便于权限管理
/// 2. 内容安全：不需要 shell 转义，避免 $ " 反引号等被 shell 解释
/// 3. 确认友好：Agent 层可以向用户展示"即将写入的文件路径和内容预览"
/// 4. 自动创建目录：如果父目录不存在，自动 mkdir -p
///
/// 确认流程在 Agent 层处理（不在此工具内），到达 execute() 时意味着已获得用户批准。

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;

use super::Tool;

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "将内容写入指定文件。如果文件已存在则覆盖，如果父目录不存在则自动创建。\
         写入操作需要用户确认。适用于：创建新文件、修改配置文件、写入代码等。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径（相对路径或绝对路径）"
                },
                "content": {
                    "type": "string",
                    "description": "要写入的文件内容"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let path_str = arguments["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let content = arguments["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("缺少 content 参数"))?;

        let path = Path::new(path_str);

        // 安全检查：禁止写入系统目录
        // 为什么在 Tool 层也做检查？双重保障——即使 Agent 层有 bug 也不会写入系统文件
        const BLOCKED_PREFIXES: &[&str] = &["/etc/", "/usr/", "/bin/", "/sbin/", "/boot/", "/sys/", "/proc/"];
        for prefix in BLOCKED_PREFIXES {
            if path_str.starts_with(prefix) {
                return Ok(format!("错误：安全限制，不允许写入系统目录 '{}'", path_str));
            }
        }

        // 如果父目录不存在，自动创建
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow::anyhow!("创建目录 '{}' 失败: {}", parent.display(), e))?;
            }
        }

        // 写入文件
        std::fs::write(path, content)
            .map_err(|e| anyhow::anyhow!("写入文件 '{}' 失败: {}", path_str, e))?;

        // 返回写入确认信息
        let line_count = content.lines().count();
        let byte_count = content.len();
        Ok(format!(
            "✓ 已写入文件 '{}' ({} 行, {} 字节)",
            path_str, line_count, byte_count
        ))
    }
}
