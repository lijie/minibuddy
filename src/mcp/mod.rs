/// MCP (Model Context Protocol) Integration
///
/// Phase 8 of mini-buddy: Support for Model Context Protocol servers.
///
/// MCP allows LLMs to interact with external tools and data sources through
/// a standardized JSON-RPC protocol. This module handles:
///
/// 1. **Transport** (`transport.rs`): JSON-RPC communication over stdio
/// 2. **Types** (`types.rs`): MCP protocol data structures
/// 3. **Server Management** (Phase 8c): Process lifecycle and tool discovery
/// 4. **Tool Adapter** (Phase 8d): Adapting MCP tools to our Tool trait
///
/// Phases:
/// - Phase 8a: Config system support ✅
/// - Phase 8b: Transport layer (this phase)
/// - Phase 8c: Server management
/// - Phase 8d: Tool adapter
/// - Phase 8e: Full integration

pub mod types;
pub mod transport;

// Re-export key types for convenience
pub use transport::McpTransport;
