/// 文件读取工具：安全地读取文件内容
///
/// 为什么单独做一个 read_file 而不是让 LLM 用 bash 的 cat？
/// 1. 语义更清晰：LLM 更容易理解"读文件"而非构造 cat 命令
/// 2. 错误处理更好：专门处理文件不存在、编码问题等，给 LLM 友好的提示
/// 3. 安全可控：Phase 4 可以加路径白名单限制（如只允许读取项目目录下的文件）
/// 4. 输出格式化：自动截断长文件、提供文件大小信息

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;

use super::Tool;

/// 文件读取工具
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "读取指定路径的文件内容。支持文本文件（代码、配置文件、文档等）。\
         如果只需要读取文件的部分内容，建议使用 bash 工具配合 head/tail/sed 命令。"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径（相对路径或绝对路径）"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        let path_str = arguments["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let path = Path::new(path_str);

        // 前置检查：给 LLM 更精确的错误提示
        if !path.exists() {
            return Ok(format!("错误：文件不存在: {}", path_str));
        }

        if !path.is_file() {
            return Ok(format!("错误：路径不是文件（可能是目录）: {}", path_str));
        }

        // 读取文件内容
        match std::fs::read_to_string(path) {
            Ok(content) => {
                // 截断过长文件，防止 token 爆炸
                // 20000 字符约 5000-10000 token
                const MAX_FILE_LEN: usize = 20000;
                if content.len() > MAX_FILE_LEN {
                    let truncated = &content[..MAX_FILE_LEN];
                    Ok(format!(
                        "{}\n\n... [文件已截断，共 {} 字节，只显示前 {} 字节]",
                        truncated,
                        content.len(),
                        MAX_FILE_LEN
                    ))
                } else {
                    Ok(content)
                }
            }
            Err(e) => {
                // 读取失败：可能是二进制文件（非 UTF-8）或权限问题
                // 返回错误信息而非 Err，让 LLM 知道发生了什么
                Ok(format!("读取文件失败: {} (可能是二进制文件或权限不足)", e))
            }
        }
    }
}
