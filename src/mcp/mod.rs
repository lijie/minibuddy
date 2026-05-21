/// MCP (Model Context Protocol) Integration
///
/// Phase 8 of mini-buddy: Support for Model Context Protocol servers.
///
/// MCP allows LLMs to interact with external tools and data sources through
/// a standardized JSON-RPC protocol. This module handles:
///
/// 1. **Transport** (`transport.rs`): JSON-RPC communication over stdio
/// 2. **Types** (`types.rs`): MCP protocol data structures
/// 3. **Server Management** (`server_manager.rs`): Process lifecycle and tool discovery
/// 4. **Tool Adapter** (`tool_adapter.rs`): Adapting MCP tools to our Tool trait
///
/// Phases:
/// - Phase 8a: Config system support ✅
/// - Phase 8b: Transport layer ✅
/// - Phase 8c: Server management ✅
/// - Phase 8d: Tool adapter ✅
/// - Phase 8e: Full integration (in progress)

pub mod types;
pub mod transport;
pub mod server_manager;
pub mod tool_adapter;

// Re-export key types and utilities for convenience
pub use transport::McpTransport;
pub use server_manager::{McpServerManager, McpServerRegistry};
pub use tool_adapter::McpToolAdapter;
