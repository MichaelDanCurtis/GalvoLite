//! MCP (Model Context Protocol) server surface.
//!
//! Exposes GalvoLite's coordination tools to spawned `claude` processes via a
//! single Streamable HTTP endpoint at `POST /mcp` (per MCP 2025-03-26).
//!
//! See `docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md`
//! §5 for the catalog and §7 for transport / auth.

pub mod dispatch;
pub mod protocol;
pub mod registry;
pub mod tools;
pub mod transport;
