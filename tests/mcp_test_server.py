#!/usr/bin/env python3
"""
极简 MCP Test Server — 用于测试 mini-buddy 的 MCP 集成

协议：JSON-RPC 2.0 over stdin/stdout
实现：initialize + tools/list + tools/call

提供两个工具：
  - echo: 原样返回输入
  - current_time: 返回当前时间

用法：
  1. chmod +x tests/mcp_test_server.py
  2. 在 ~/.mini-buddy/config.toml 中添加：
     [mcp.test]
     command = "python3"
     args = ["tests/mcp_test_server.py"]
  3. cargo run
  4. 问 Agent: "用 echo 工具重复一下 hello world"
"""

import json
import sys
from datetime import datetime


def send_response(response):
    """发送 JSON-RPC 响应到 stdout"""
    line = json.dumps(response)
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def handle_initialize(req_id, params):
    """处理 initialize 握手"""
    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "mcp-test-server",
                "version": "0.1.0"
            }
        }
    }


def handle_tools_list(req_id):
    """返回可用工具列表"""
    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "result": {
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo back the input message. Useful for testing MCP connectivity.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "message": {
                                "type": "string",
                                "description": "The message to echo back"
                            }
                        },
                        "required": ["message"]
                    }
                },
                {
                    "name": "current_time",
                    "description": "Get the current date and time.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                }
            ]
        }
    }


def handle_tools_call(req_id, params):
    """执行工具调用"""
    tool_name = params.get("name", "")
    arguments = params.get("arguments", {})

    if tool_name == "echo":
        message = arguments.get("message", "(empty)")
        content = [{"type": "text", "text": f"Echo: {message}"}]
    elif tool_name == "current_time":
        now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        content = [{"type": "text", "text": f"Current time: {now}"}]
    else:
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "content": [{"type": "text", "text": f"Unknown tool: {tool_name}"}],
                "isError": True
            }
        }

    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "result": {
            "content": content,
            "isError": False
        }
    }


def main():
    """主循环：读取 stdin 的 JSON-RPC 请求，处理后写回 stdout"""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        req_id = request.get("id")
        method = request.get("method", "")
        params = request.get("params", {})

        if method == "initialize":
            response = handle_initialize(req_id, params)
        elif method == "notifications/initialized":
            continue  # 通知无需响应
        elif method == "tools/list":
            response = handle_tools_list(req_id)
        elif method == "tools/call":
            response = handle_tools_call(req_id, params)
        else:
            response = {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {
                    "code": -32601,
                    "message": f"Method not found: {method}"
                }
            }

        send_response(response)


if __name__ == "__main__":
    main()
