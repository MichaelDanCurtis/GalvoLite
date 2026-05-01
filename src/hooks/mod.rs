//! Hook surface: shared types and the `galvolite-hook` binary's pure logic.
//!
//! See spec §6 (`docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md`)
//! for the wire format. The `client` submodule is exercised both by the
//! binary at `src/bin/galvolite-hook.rs` and by unit tests via `mockito`.

pub mod client;
pub mod marker;
pub mod types;
