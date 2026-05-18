/// Bash 工具：执行终端命令
///
/// 这是 Agent 最强大的工具——能执行任意 shell 命令，
/// 让 Agent 可以查看文件系统、运行程序、搜索代码等。
///
/// 安全考虑：
/// - Phase 3 教学版本不做限制，让学习者看到完整的工具执行流程
/// - Phase 4 将引入沙盒机制：命令黑名单 + 权限分级（读自动/写确认/危险拦截）
///
/// 为什么用同步的 std::process::Command 而不是 tokio::process::Command？
/// - 教学项目中命令执行时间短，同步足够
/// - 避免引入额外的异步复杂度
/// - Phase 11 引入超时控制时会改用 tokio::process + timeout

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;

use super::Tool;

/// Bash 工具结构体
///
/// 为什么是零大小结构体（unit struct）？
/// 因为 Bash 工具不需要持有任何状态——每次执行都是独立的。
/// 未来如果需要配置（如工作目录、超时时间），可以添加字段。
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "执行终端命令。可以运行任何 bash 命令，如 ls、cat、grep、find 等。\
         适用于：查看目录结构、搜索文件、执行程序、查看系统信息等。\
         返回命令的标准输出和标准错误。"
    }

    fn parameters_schema(&self) -> Value {
        // JSON Schema 格式：描述参数的类型和约束
        // LLM 会根据这个 schema 生成合法的参数 JSON
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "要执行的 bash 命令"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<String> {
        // 从参数中提取 command 字段
        let command = arguments["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("缺少 command 参数"))?;

        // 使用 sh -c 执行命令
        // 为什么用 sh -c 而不是直接 Command::new(program)？
        // 因为 LLM 生成的是完整的 shell 命令字符串，可能包含管道符、重定向等 shell 语法
        // 例如 "ls -la | grep .rs" 需要 shell 来解释管道符
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| anyhow::anyhow!("执行命令失败: {}", e))?;

        // 合并 stdout 和 stderr
        // 为什么都要？因为很多有用信息在 stderr（如 git、cargo 的进度信息）
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut result = String::new();

        if !stdout.is_empty() {
            result.push_str(&stdout);
        }

        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("[stderr] ");
            result.push_str(&stderr);
        }

        // 命令执行失败时（非零退出码），附加退出码信息
        // 为什么不 return Err？因为 LLM 需要看到错误信息来调整策略，
        // 比如文件不存在时 LLM 可以尝试其他路径
        if !output.status.success() {
            if result.is_empty() {
                result = format!("命令执行失败，退出码: {}", output.status);
            } else {
                result.push_str(&format!("\n[退出码: {}]", output.status));
            }
        }

        // 截断过长输出，防止 token 用量爆炸
        // 10000 字符约 3000-5000 token，足够看到关键信息
        const MAX_OUTPUT_LEN: usize = 10000;
        if result.len() > MAX_OUTPUT_LEN {
            result.truncate(MAX_OUTPUT_LEN);
            result.push_str("\n... [输出已截断]");
        }

        Ok(result)
    }
}
